use crate::types::ChatMessage;
use tracing::{error, info};
use super::NetworkEvents;

impl<'a> NetworkEvents<'a> {

    pub(super) async fn broadcast_chat_message(&mut self, message: ChatMessage) {

        info!("{}Broadcasting message {:?}", self.nname(), message);

        // testing TEE
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
                error!("{} Err: topic does not exist {:?}", self.nname(), message.topic);
                info!("Topics subscribed:");
                let _ = self.swarm.behaviour_mut().gossipsub.topics()
                    .into_iter()
                    .map(|t| println!("{:?}", t)).collect::<()>();
            }
        }
    }
}