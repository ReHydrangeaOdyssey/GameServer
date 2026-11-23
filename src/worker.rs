use crate::rpc::Failure;
use std::collections::BTreeMap;
use tokio::sync::{mpsc, oneshot};
type Command<T> = Box<dyn FnOnce(&mut BTreeMap<u64, T>) + Send>;
pub struct Workers<T> {
    queues: Vec<mpsc::Sender<Command<T>>>,
}
impl<T: Send + 'static> Workers<T> {
    pub fn new(count: usize, queue_limit: usize) -> Result<Self, crate::transport::Error> {
        if count == 0 || queue_limit == 0 {
            return Err("worker and queue counts must be positive".into());
        }
        let mut queues = Vec::new();
        for index in 0..count {
            let (send, mut receive) = mpsc::channel::<Command<T>>(queue_limit);
            std::thread::Builder::new()
                .name(format!("guild-battle-{index}"))
                .spawn(move || {
                    let mut battles = BTreeMap::new();
                    while let Some(command) = receive.blocking_recv() {
                        command(&mut battles);
                    }
                })?;
            queues.push(send);
        }
        Ok(Self { queues })
    }
    pub async fn execute<R: Send + 'static>(
        &self,
        worker: usize,
        function: impl FnOnce(&mut BTreeMap<u64, T>) -> Result<R, Failure> + Send + 'static,
    ) -> Result<R, Failure> {
        let queue = self.queues.get(worker).ok_or(Failure::UNAVAILABLE)?;
        let (sender, receiver) = oneshot::channel();
        // Bounded FIFO dispatch preserves receipt order and applies backpressure without dropping work.
        queue
            .send(Box::new(move |state| {
                let result = function(state);
                let _closed = sender.send(result);
            }))
            .await
            .map_err(|_| Failure::UNAVAILABLE)?;
        receiver.await.map_err(|_| Failure::UNAVAILABLE)?
    }
}
