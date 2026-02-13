use crate::types::Message;
use std::sync::Arc;
use tokio::sync::broadcast;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct Topic {
    #[allow(dead_code)]
    pub id: String,
    pub tx: broadcast::Sender<Arc<Message>>,
    #[allow(dead_code)]
    pub last_access: SystemTime,
}

impl Topic {
    pub fn new(id: String) -> Self {
        let (tx, _) = broadcast::channel(1024); // Buffer size 1024
        Self {
            id,
            tx,
            last_access: SystemTime::now(),
        }
    }

    pub fn touch(&mut self) {
        self.last_access = SystemTime::now();
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Message>> {
        self.tx.subscribe()
    }

    pub fn publish(&self, msg: Arc<Message>) -> usize {
        // Returns number of subscribers
        self.tx.send(msg).unwrap_or(0)
    }
}
