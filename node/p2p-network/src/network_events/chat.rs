use crate::types::ChatMessage;
use super::NetworkEvents;

impl<'a> NetworkEvents<'a> {

    pub(super) async fn broadcast_chat_message(&mut self, message: ChatMessage) {

        self.log(format!("Broadcasting message: {:?}", message));

        /// testing TEE
        let (
            _tee_quote ,
            _tee_quote_bytes
        ) = runtime::tee_attestation::generate_tee_attestation(true)
            .expect("tee attestation generation err");

        match self.topics.get(&message.topic.to_string()) {
            Some(topic) => {

                match self.swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic.clone(), message.message) {
                        Ok(_) => {}
                        Err(e) => println!("Publish err: {:?}", e),
                    }
            }
            None => {
                self.log(format!("Err: topic does not exist {:?}", message.topic));
                self.log(format!("Topics subscribed:"));
                let _ = self.swarm.behaviour_mut().gossipsub.topics()
                    .into_iter()
                    .map(|t| println!("{:?}", t)).collect::<()>();
            }
        }
    }
}