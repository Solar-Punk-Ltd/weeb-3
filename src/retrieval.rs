// #![allow(warnings)]

use crate::{
    apply_credit,
    // // // // // // // //
    cancel_reserve,
    // // // // // // // //
    get_proximity,
    // // // // // // // //
    mpsc,
    // // // // // // // //
    price,
    // // // // // // // //
    reserve,
    // // // // // // // //
    retrieve_handler,
    // // // // // // // //
    stream,
    // // // // // // // //
    valid_cac,
    // // // // // // // //
    valid_soc,
    // // // // // // // //
    Date,
    // // // // // // // //
    Duration,
    // // // // // // // //
    HashMap,
    // // // // // // // //
    JsValue,
    // // // // // // // //
    Mutex,
    // // // // // // // //
    PeerAccounting,
    // // // // // // // //
    PeerId,
    // // // // // // // //
    RETRIEVE_ROUND_TIME,
    // // // // // // // //
};

use serde_json::Value;

pub async fn retrieve_chunk(
    chunk_address: &Vec<u8>,
    control: &mut stream::Control,
    peers: &Mutex<HashMap<String, PeerId>>,
    accounting: &Mutex<HashMap<PeerId, Mutex<PeerAccounting>>>,
    refresh_chan: &mpsc::Sender<(PeerId, u64)>,
) -> Vec<u8> {
    let timestart = Date::now();
    let mut skiplist: HashMap<PeerId, _> = HashMap::new();
    let mut overdraftlist: HashMap<PeerId, _> = HashMap::new();

    let mut closest_overlay = "".to_string();
    let mut closest_peer_id = libp2p::PeerId::random();

    let mut selected = false;
    let mut round_commence = Date::now();
    let mut current_max_po = 0;
    let mut error_count = 0;
    let mut max_error = 8;

    let mut cd = vec![];

    while error_count < max_error {
        let mut seer = true;

        while seer {
            closest_overlay = "".to_string();
            closest_peer_id = libp2p::PeerId::random();
            current_max_po = 0;
            selected = false;
            {
                let peers_map = peers.lock().unwrap();
                for (ov, id) in peers_map.iter() {
                    if skiplist.contains_key(id) {
                        continue;
                    }
                    let current_po = get_proximity(&chunk_address, &hex::decode(&ov).unwrap());

                    if current_po >= current_max_po {
                        selected = true;
                        closest_overlay = ov.clone();
                        closest_peer_id = id.clone();
                        current_max_po = current_po;
                    }
                }
            }
            if selected {
                skiplist.insert(closest_peer_id, "");
                web_sys::console::log_1(&JsValue::from(format!(
                    "Selected peer {:#?}!",
                    closest_peer_id
                )));
            } else {
                if overdraftlist.is_empty() {
                    return vec![];
                } else {
                    for (k, _v) in overdraftlist.iter() {
                        let _ =
                            refresh_chan.send((k.clone(), 10 * crate::accounting::REFRESH_RATE));
                        skiplist.remove(k);
                    }
                    overdraftlist.clear();

                    let round_now = Date::now();

                    let seg = round_now - round_commence;
                    if seg < RETRIEVE_ROUND_TIME {
                        web_sys::console::log_1(&JsValue::from(format!(
                            "Ease retrieve overdraft retries loop for {}",
                            RETRIEVE_ROUND_TIME - seg
                        )));
                        async_std::task::sleep(Duration::from_millis(
                            (RETRIEVE_ROUND_TIME - seg) as u64,
                        ))
                        .await;
                    }

                    round_commence = Date::now();

                    continue;
                }
            }

            let req_price = price(&closest_overlay, &chunk_address);

            web_sys::console::log_1(&JsValue::from(format!(
                "Attempt to reserve price {:#?} for chunk {:#?} from peer {:#?}!",
                req_price, chunk_address, closest_peer_id
            )));

            {
                let accounting_peers = accounting.lock().unwrap();
                if max_error > accounting_peers.len() {
                    max_error = accounting_peers.len();
                };
                if accounting_peers.contains_key(&closest_peer_id) {
                    let accounting_peer = accounting_peers.get(&closest_peer_id).unwrap();
                    let allowed = reserve(accounting_peer, req_price, refresh_chan);
                    if !allowed {
                        web_sys::console::log_1(&JsValue::from(format!(
                            "Overdraft for peer {}",
                            closest_peer_id
                        )));
                        overdraftlist.insert(closest_peer_id, "");
                    } else {
                        web_sys::console::log_1(&JsValue::from(format!(
                            "Selected peer with successful reserve {}!",
                            closest_peer_id
                        )));
                        seer = false;
                    }
                }
            }
        }

        let req_price = price(&closest_overlay, &chunk_address);

        let (chunk_out, chunk_in) = mpsc::channel::<Vec<u8>>();

        web_sys::console::log_1(&JsValue::from(format!(
            "Actually retrieving for peer {}!",
            closest_peer_id
        )));

        retrieve_handler(closest_peer_id, chunk_address.clone(), control, &chunk_out).await;

        let chunk_data = chunk_in.try_recv();
        if !chunk_data.is_err() {
            let accounting_peers = accounting.lock().unwrap();
            if accounting_peers.contains_key(&closest_peer_id) {
                let accounting_peer = accounting_peers.get(&closest_peer_id).unwrap();
                apply_credit(accounting_peer, req_price);
            }
        } else {
            let accounting_peers = accounting.lock().unwrap();
            if accounting_peers.contains_key(&closest_peer_id) {
                let accounting_peer = accounting_peers.get(&closest_peer_id).unwrap();
                cancel_reserve(accounting_peer, req_price)
            }
        }

        cd = match chunk_data {
            Ok(ref x) => x.clone(),
            Err(_x) => {
                error_count += 1;
                vec![]
            }
        };

        // chan send?

        match chunk_data {
            Ok(_x) => {
                break;
            }
            _ => {}
        };
    }

    if cd.len() > 0 {
        web_sys::console::log_1(&JsValue::from(format!(
            "Successfully retrieved chunk {:#?} from peer {:#?}!",
            cd, closest_peer_id
        )));
    }

    let timeend = Date::now();

    web_sys::console::log_1(&JsValue::from(format!(
        "Retrieve time duration {} ms!",
        timeend - timestart
    )));

    web_sys::console::log_1(&JsValue::from(format!(
        "Chunk content: {:#?} ",
        String::from_utf8(cd.clone())
    )));

    let contaddrd = valid_cac(&cd, chunk_address);
    if !contaddrd {
        let socd = valid_soc(&cd, chunk_address);
        if !socd {
            return vec![];
        }
    }

    let obfuscation_key = &cd[8..40];
    web_sys::console::log_1(&JsValue::from(format!(
        "obfuscation_key: {:#?} ",
        obfuscation_key
    )));
    let mf_version = &cd[40..71];
    web_sys::console::log_1(&JsValue::from(format!("mf_version: {:#?} ", mf_version)));

    let ref_size = &cd[71];
    web_sys::console::log_1(&JsValue::from(format!("ref_size: {:#?} ", ref_size)));

    let ref_delimiter = (72 + ref_size) as usize;
    let actual_reference = &cd[72..ref_delimiter];
    web_sys::console::log_1(&JsValue::from(format!(
        "actual_reference: {:#?}",
        actual_reference
    )));

    let index_delimiter = (ref_delimiter + 32) as usize;
    let index = &cd[ref_delimiter..index_delimiter];
    web_sys::console::log_1(&JsValue::from(format!("index: {:#?}", index)));

    // fork parts

    let mut fork_start_current = index_delimiter;

    let fork_type = &cd[fork_start_current];
    web_sys::console::log_1(&JsValue::from(format!("fork_type: {:#?}", fork_type)));

    let fork_prefix_length = &cd[fork_start_current + 1];
    web_sys::console::log_1(&JsValue::from(format!(
        "fork_prefix_length: {:#?}",
        fork_prefix_length
    )));

    let fork_prefix_delimiter = fork_start_current + 2 + 30;
    let fork_prefix = &cd[fork_start_current + 2..fork_prefix_delimiter];
    web_sys::console::log_1(&JsValue::from(format!("fork_prefix: {:#?}", fork_prefix)));

    let fork_reference = &cd[fork_prefix_delimiter..fork_prefix_delimiter + 32];
    web_sys::console::log_1(&JsValue::from(format!(
        "fork_reference: {:#?}",
        fork_reference
    )));

    let fork_metadata_bytesize: [u8; 2] = cd
        [fork_prefix_delimiter + 32..fork_prefix_delimiter + 34]
        .try_into()
        .unwrap();
    web_sys::console::log_1(&JsValue::from(format!(
        "fork_metadata_bytesize: {:#?}",
        fork_metadata_bytesize
    )));

    let calc = u16::from_be_bytes(fork_metadata_bytesize) as usize;

    let fork_metadata_delimiter = fork_prefix_delimiter + 34 + calc;
    let fork_metadata = &cd[fork_prefix_delimiter + 34..fork_metadata_delimiter];
    web_sys::console::log_1(&JsValue::from(format!(
        "fork_metadata: {:#?}",
        fork_metadata
    )));
    let v0: Value = serde_json::from_slice(fork_metadata).unwrap_or("nil".into());

    // let mut v = Manifest::deserialize(MapDeserializer::new(stuff.into_iter())).unwrap();

    web_sys::console::log_1(&JsValue::from(format!("Chunk content deser: {:#?} ", v0)));

    return cd;
}

// 3ab408eea4f095bde55c1caeeac8e7fcff49477660f0a28f652f0a6d9c60d05f
