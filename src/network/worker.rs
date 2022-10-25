use super::message::Message;
use super::peer;
use super::server::Handle as ServerHandle;
use crate::types::block::Block;
use crate::types::hash::{H256, Hashable};
use crate::blockchain::Blockchain;
use std::thread;
use std::sync::{Arc, Mutex};

use log::{debug, warn, error};

#[cfg(any(test,test_utilities))]
use super::peer::TestReceiver as PeerTestReceiver;
#[cfg(any(test,test_utilities))]
use super::server::TestReceiver as ServerTestReceiver;
#[derive(Clone)]
pub struct Worker {
    msg_chan: smol::channel::Receiver<(Vec<u8>, peer::Handle)>,
    num_worker: usize,
    server: ServerHandle,
    arc_mutex: Arc<Mutex<Blockchain>>, 
}


impl Worker {
    pub fn new(
        num_worker: usize,
        msg_src: smol::channel::Receiver<(Vec<u8>, peer::Handle)>,
        server: &ServerHandle,
        arc_mutex: &Arc<Mutex<Blockchain>>, 
    ) -> Self {
        Self {
            msg_chan: msg_src,
            num_worker,
            server: server.clone(),
            arc_mutex: arc_mutex.clone()
        }
    }

    pub fn start(self) {
        let num_worker = self.num_worker;
        for i in 0..num_worker {
            let cloned = self.clone();
            thread::spawn(move || {
                cloned.worker_loop();
                warn!("Worker thread {} exited", i);
            });
        }
    }

    fn worker_loop(&self) {
        loop {
            let result = smol::block_on(self.msg_chan.recv());
            if let Err(e) = result {
                error!("network worker terminated {}", e);
                break;
            }

            let msg = result.unwrap();
            let (msg, mut peer) = msg;
            let msg: Message = bincode::deserialize(&msg).unwrap();
            match msg {
                Message::Ping(nonce) => {
                    debug!("Ping: {}", nonce);
                    peer.write(Message::Pong(nonce.to_string()));
                }
                Message::Pong(nonce) => {
                    debug!("Pong: {}", nonce);
                }
                Message::NewBlockHashes(hashvec) => {
                    let mut new_hashes = Vec::<H256>::new();
                    for hash in hashvec {
                        if !&self.arc_mutex.lock().unwrap().hash_map.contains_key(&hash) {
                            new_hashes.push(hash);
                        }
                    }
                    if new_hashes.len() > 0 {
                        peer.write(Message::GetBlocks(new_hashes));
                    }
                }
                Message::GetBlocks(hashvec) => {
                    let mut blocks = Vec::new();
                    
                    for hash in hashvec {
                        if self.arc_mutex.lock().unwrap().hash_map.contains_key(&hash){ 
                            let blockchain = self.arc_mutex.lock().unwrap();
                            let block_response = blockchain.hash_map.get(&hash);
                            let block_option = Option::expect(block_response, "block not found");
                            blocks.push(block_option.clone());
                        } 
                    }
                    peer.write(Message::Blocks(blocks));
                }
                Message::Blocks(blockvec) => {
                    let mut new_hashes = Vec::<H256>::new();
                    for block in blockvec {
                        let block_hash = block.hash();
                        if self.arc_mutex.lock().unwrap().hash_map.contains_key(&block_hash) == false {
                            self.arc_mutex.lock().unwrap().insert(&block);
                            new_hashes.push(block_hash);
                        }
                    }
                    self.server.broadcast(Message::NewBlockHashes(new_hashes));
                }
                Message::NewTransactionHashes(_) => todo!(),
                Message::GetTransactions(_) => todo!(),
                Message::Transactions(_) => todo!(),
            }
        }
    }
}

#[cfg(any(test,test_utilities))]
struct TestMsgSender {
    s: smol::channel::Sender<(Vec<u8>, peer::Handle)>
}
#[cfg(any(test,test_utilities))]
impl TestMsgSender {
    fn new() -> (TestMsgSender, smol::channel::Receiver<(Vec<u8>, peer::Handle)>) {
        let (s,r) = smol::channel::unbounded();
        (TestMsgSender {s}, r)
    }

    fn send(&self, msg: Message) -> PeerTestReceiver {
        let bytes = bincode::serialize(&msg).unwrap();
        let (handle, r) = peer::Handle::test_handle();
        smol::block_on(self.s.send((bytes, handle))).unwrap();
        r
    }
}
#[cfg(any(test,test_utilities))]
/// returns two structs used by tests, and an ordered vector of hashes of all blocks in the blockchain
fn generate_test_worker_and_start() -> (TestMsgSender, ServerTestReceiver, Vec<H256>) {

    let (server, server_receiver) = ServerHandle::new_for_test();
    let (test_msg_sender, msg_chan) = TestMsgSender::new();
    let new_blockchain= &Arc::new(Mutex::new(Blockchain::new()));
    let worker = Worker::new(1, msg_chan, &server, new_blockchain);
    worker.start(); 
    // generate and append the hash of the genesis block
    let blockchain_vector = new_blockchain.lock().unwrap().all_blocks_in_longest_chain();
    (test_msg_sender, server_receiver, blockchain_vector)
}

// DO NOT CHANGE THIS COMMENT, IT IS FOR AUTOGRADER. BEFORE TEST

#[cfg(test)]
mod test {
    use ntest::timeout;
    use crate::types::block::generate_random_block;
    use crate::types::hash::Hashable;

    use super::super::message::Message;
    use super::generate_test_worker_and_start;

    #[test]
    #[timeout(60000)]
    fn reply_new_block_hashes() {
        let (test_msg_sender, _server_receiver, v) = generate_test_worker_and_start();
        let random_block = generate_random_block(v.last().unwrap());
        let mut peer_receiver = test_msg_sender.send(Message::NewBlockHashes(vec![random_block.hash()]));
        let reply = peer_receiver.recv();
        if let Message::GetBlocks(v) = reply {
            assert_eq!(v, vec![random_block.hash()]);
        } else {
            panic!();
        }
    }
    #[test]
    #[timeout(60000)]
    fn reply_get_blocks() {
        let (test_msg_sender, _server_receiver, v) = generate_test_worker_and_start();
        let h = v.last().unwrap().clone();
        let mut peer_receiver = test_msg_sender.send(Message::GetBlocks(vec![h.clone()]));
        let reply = peer_receiver.recv();
        if let Message::Blocks(v) = reply {
            assert_eq!(1, v.len());
            assert_eq!(h, v[0].hash())
        } else {
            panic!();
        }
    }
    #[test]
    #[timeout(60000)]
    fn reply_blocks() {
        let (test_msg_sender, server_receiver, v) = generate_test_worker_and_start();
        let random_block = generate_random_block(v.last().unwrap());
        let mut _peer_receiver = test_msg_sender.send(Message::Blocks(vec![random_block.clone()]));
        let reply = server_receiver.recv().unwrap();
        if let Message::NewBlockHashes(v) = reply {
            assert_eq!(v, vec![random_block.hash()]);
        } else {
            panic!();
        }
    }
}

// DO NOT CHANGE THIS COMMENT, IT IS FOR AUTOGRADER. AFTER TEST