use libp2p::request_response;
use colored::Colorize;
use crate::SendError;
use crate::types::{
    NetworkLoopEvent,
    FragmentRequestEnum,
    FragmentResponseEnum
};
use super::EventLoop;


//// Request Response Protocol
impl<'a> EventLoop<'a> {
    pub(super) async fn handle_request_response(
        &mut self,
        event: request_response::Event<FragmentRequestEnum, FragmentResponseEnum>,
    ) {
        match event {
            request_response::Event::Message { message, .. } => {

                match message {

                    //////////////////////////////////
                    // Inbound Requests
                    //////////////////////////////////
                    request_response::Message::Request {
                        request: fragment_request,
                        channel,
                        ..
                    } => {
                        match fragment_request {
                            FragmentRequestEnum::FragmentRequest(
                                agent_name_nonce,
                                frag_num,
                                sender_peer_id
                            ) => {
                                self.network_event_sender
                                    .send(NetworkLoopEvent::InboundCapsuleFragRequest {
                                        agent_name_nonce,
                                        frag_num,
                                        sender_peer_id,
                                        channel,
                                    })
                                    .await
                                    .expect("Event receiver not to be dropped.");
                            },
                            FragmentRequestEnum::ProvidingFragment(
                                agent_name_nonce,
                                frag_num,
                                sender_peer_id
                            ) => {
                                self.network_event_sender
                                    .send(NetworkLoopEvent::SaveKfragProviderRequest {
                                        agent_name_nonce,
                                        frag_num,
                                        sender_peer_id,
                                        channel,
                                    })
                                    .await
                                    .expect("Event receiver not to be dropped.");
                            }
                        }
                    }

                    //////////////////////////////////
                    // Inbound Responses
                    //////////////////////////////////
                    request_response::Message::Response {
                        request_id,
                        response,
                    } => {
                        match response {
                            FragmentResponseEnum::FragmentResponse(fragment_bytes) => {
                                // get sender channel associated with the request-response id
                                let sender = self.pending.request_fragments
                                    .remove(&request_id)
                                    .expect("request_response: Request pending.");
                                // send fragment to it
                                let _ = sender.send(fragment_bytes);
                            }
                            FragmentResponseEnum::KfragProviderAck => {
                                self.log(format!("\nVessel acknowledged fragment provider\n").green());
                            }
                            _ => {}
                        }
                    }
                }
            },
            request_response::Event::ResponseSent { .. } => {}
            request_response::Event::InboundFailure { .. } => {}
            request_response::Event::OutboundFailure {
                request_id,
                error,
                peer
            } => {
                self.pending.request_fragments
                    .remove(&request_id)
                    .expect(&format!("RequestId {} not found for {}", request_id, peer))
                    .send(Err(SendError(error.to_string())))
                    .ok();
            }
        }
    }
}

