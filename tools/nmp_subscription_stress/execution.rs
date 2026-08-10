use nmp::mechanism::core::{
    Effect, EngineCore, ObservationFact, ObservationId, RequestAttemptId, RequestHandoffOutcome,
};
use nmp_grammar::{ConcreteFilter, DescriptorHash, RelaySessionKey};
use nmp_router::{SubId, WireOp};
use nmp_store::RedbStore;
use nmp_transport::{RelayFrame, RelayHandle};
use nostr::{RelayMessage, SubscriptionId};

#[derive(Clone)]
pub(crate) struct WireRequest {
    pub(crate) session: RelaySessionKey,
    pub(crate) sub_id: SubId,
    pub(crate) filter: ConcreteFilter,
    pub(crate) attempt_id: RequestAttemptId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RelayRequestWitness {
    pub(crate) observation: ObservationId,
    pub(crate) path: String,
    pub(crate) filter_revision: u64,
    pub(crate) filter: ConcreteFilter,
    pub(crate) replay: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestSettledWitness {
    pub(crate) observation: ObservationId,
    pub(crate) path: String,
    pub(crate) filter_revision: u64,
    pub(crate) request_revision: u64,
}

pub(crate) fn wire_requests(effects: &[Effect]) -> Vec<WireRequest> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Wire(delta) => Some(delta),
            _ => None,
        })
        .flat_map(|delta| {
            delta.ops.iter().flat_map(move |(session, operations)| {
                operations
                    .iter()
                    .filter_map(move |operation| match operation {
                        WireOp::Req(sub_id, filter) => Some(WireRequest {
                            session: session.clone(),
                            sub_id: sub_id.clone(),
                            filter: filter.clone(),
                            attempt_id: delta.attempt_id(session, sub_id, filter),
                        }),
                        WireOp::Close(_) => None,
                    })
            })
        })
        .collect()
}

pub(crate) fn accept_requests(
    core: &mut EngineCore<RedbStore>,
    requests: &[WireRequest],
    generation: u64,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    for request in requests {
        effects.extend(
            core.on_wire_request_handoff(RequestHandoffOutcome::Accepted {
                attempt_id: request.attempt_id,
                handle: RelayHandle {
                    slot: 0,
                    generation,
                },
            }),
        );
    }
    effects
}

pub(crate) fn eose_request(
    core: &mut EngineCore<RedbStore>,
    request: &WireRequest,
    slot: u32,
    generation: u64,
) -> Vec<Effect> {
    core.handle(nmp::mechanism::core::EngineMsg::RelayFrame(
        RelayHandle { slot, generation },
        request.session.clone(),
        RelayFrame::from(RelayMessage::eose(SubscriptionId::new(
            request.sub_id.1.to_string(),
        ))),
    ))
}

pub(crate) fn relay_request_witnesses(effects: &[Effect]) -> Vec<RelayRequestWitness> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::EmitObservationEvidence(observation, evidence) => {
                Some((*observation, evidence))
            }
            _ => None,
        })
        .flat_map(|(observation, evidence)| {
            evidence
                .iter()
                .filter_map(move |evidence| match &evidence.fact {
                    ObservationFact::RelayRequest {
                        path,
                        filter_revision,
                        filter,
                        replay,
                        ..
                    } => Some(RelayRequestWitness {
                        observation,
                        path: path.clone(),
                        filter_revision: *filter_revision,
                        filter: filter.as_ref().clone(),
                        replay: *replay,
                    }),
                    _ => None,
                })
        })
        .collect()
}

pub(crate) fn request_settled_witnesses(effects: &[Effect]) -> Vec<RequestSettledWitness> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::EmitObservationEvidence(observation, evidence) => {
                Some((*observation, evidence))
            }
            _ => None,
        })
        .flat_map(|(observation, evidence)| {
            evidence
                .iter()
                .filter_map(move |evidence| match &evidence.fact {
                    ObservationFact::RequestSettled {
                        path,
                        filter_revision,
                        request_revision,
                        ..
                    } => Some(RequestSettledWitness {
                        observation,
                        path: path.clone(),
                        filter_revision: *filter_revision,
                        request_revision: *request_revision,
                    }),
                    _ => None,
                })
        })
        .collect()
}

pub(crate) fn concrete_revisions(
    effects: &[Effect],
) -> Vec<(ObservationId, String, u64, DescriptorHash)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::EmitObservationEvidence(observation, evidence) => {
                Some((*observation, evidence))
            }
            _ => None,
        })
        .flat_map(|(observation, evidence)| {
            evidence
                .iter()
                .flat_map(move |evidence| match &evidence.fact {
                    ObservationFact::ConcreteFilter {
                        path,
                        revision,
                        filters,
                        ..
                    } => filters
                        .iter()
                        .map(|filter| (observation, path.clone(), *revision, filter.hash()))
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                })
        })
        .collect()
}
