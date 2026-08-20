use std::collections::{BTreeMap, BTreeSet};

use nmp::{AcquisitionEvidence, Row, RowDelta};
use nostr::EventId;

#[derive(Default)]
pub(super) struct RowViews {
    expected_generations: BTreeMap<String, u64>,
    observations: BTreeMap<String, ObservationRows>,
    rows: BTreeMap<EventId, OwnedRow>,
}

struct ObservationRows {
    generation: u64,
    ids: BTreeSet<EventId>,
    evidence: Vec<AcquisitionEvidence>,
}

struct OwnedRow {
    row: Row,
    observations: BTreeSet<String>,
}

#[derive(Clone)]
pub(super) struct ViewRow {
    pub(super) row: Row,
}

impl RowViews {
    pub(super) fn apply_frame(
        &mut self,
        observation_id: &str,
        generation: u64,
        deltas: Vec<RowDelta>,
        evidence: Vec<AcquisitionEvidence>,
        recent_rows: &crate::recent_rows::RecentRows,
    ) -> super::RowTransition {
        let expected = self
            .expected_generations
            .entry(observation_id.to_string())
            .or_insert(generation);
        if generation < *expected {
            return super::RowTransition::default();
        }
        *expected = generation;
        let current_generation = self
            .observations
            .get(observation_id)
            .map(|view| view.generation);
        if current_generation.is_some_and(|current| generation < current) {
            return super::RowTransition::default();
        }
        if current_generation != Some(generation) {
            return self.replace_generation(
                observation_id,
                generation,
                deltas,
                evidence,
                recent_rows,
            );
        }

        let mut transition = super::RowTransition::default();
        for id in deltas.iter().filter_map(|delta| match delta {
            RowDelta::Removed(id) => Some(*id),
            _ => None,
        }) {
            self.detach(observation_id, id, &mut transition, recent_rows);
        }
        for delta in deltas {
            match delta {
                RowDelta::Added(row) | RowDelta::Updated(row) => {
                    self.attach(observation_id, row, &mut transition, recent_rows)
                }
                RowDelta::SourcesGrew { id, sources } => {
                    if let Some(owned) = self.rows.get_mut(&id) {
                        owned.row.sources = sources;
                    }
                }
                RowDelta::Removed(_) => {}
            }
        }
        if let Some(view) = self.observations.get_mut(observation_id) {
            view.evidence = evidence;
        }
        transition
    }

    pub(super) fn begin_observation(&mut self, observation_id: &str, generation: u64) {
        let expected = self
            .expected_generations
            .entry(observation_id.to_string())
            .or_default();
        *expected = (*expected).max(generation);
    }

    pub(super) fn close(
        &mut self,
        observation_id: &str,
        generation: u64,
        recent_rows: &crate::recent_rows::RecentRows,
    ) -> super::RowTransition {
        if self.expected_generations.get(observation_id) != Some(&generation) {
            return super::RowTransition::default();
        }
        self.expected_generations.remove(observation_id);
        let Some(view) = self.observations.get(observation_id) else {
            return super::RowTransition::default();
        };
        if view.generation > generation {
            return super::RowTransition::default();
        }
        let ids = self
            .observations
            .remove(observation_id)
            .expect("observation existed")
            .ids;
        let mut transition = super::RowTransition::default();
        for id in ids {
            self.detach_owner_only(observation_id, id, &mut transition, recent_rows);
        }
        transition
    }

    pub(super) fn row(&self, id: &EventId) -> Option<ViewRow> {
        self.rows.get(id).map(|owned| ViewRow {
            row: owned.row.clone(),
        })
    }

    /// Rows owned by one retained observation, irrespective of whether another
    /// observation also owns the same canonical NMP row.
    pub(super) fn observation_rows(&self) -> Vec<(String, ViewRow)> {
        self.observations
            .iter()
            .flat_map(|(observation_id, observation)| {
                self.rows_for_ids(Some(&observation.ids))
                    .into_iter()
                    .map(|row| (observation_id.clone(), row))
            })
            .collect()
    }

    fn replace_generation(
        &mut self,
        observation_id: &str,
        generation: u64,
        deltas: Vec<RowDelta>,
        evidence: Vec<AcquisitionEvidence>,
        recent_rows: &crate::recent_rows::RecentRows,
    ) -> super::RowTransition {
        let next_rows = deltas
            .into_iter()
            .filter_map(|delta| match delta {
                RowDelta::Added(row) => Some((row.id(), row)),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let old_ids = self
            .observations
            .remove(observation_id)
            .map(|view| view.ids)
            .unwrap_or_default();
        let ids = next_rows.keys().copied().collect::<BTreeSet<_>>();
        let mut transition = super::RowTransition::default();
        for id in old_ids.difference(&ids) {
            self.detach_owner_only(observation_id, *id, &mut transition, recent_rows);
        }
        self.observations.insert(
            observation_id.to_string(),
            ObservationRows {
                generation,
                ids: BTreeSet::new(),
                evidence,
            },
        );
        for (_, row) in next_rows {
            self.attach(observation_id, row, &mut transition, recent_rows);
        }
        debug_assert_eq!(self.observations[observation_id].ids, ids);
        transition
    }

    fn attach(
        &mut self,
        observation_id: &str,
        row: Row,
        transition: &mut super::RowTransition,
        recent_rows: &crate::recent_rows::RecentRows,
    ) {
        let id = row.id();
        let entered = self
            .observations
            .get_mut(observation_id)
            .expect("observation is installed before its rows")
            .ids
            .insert(id);
        if entered {
            transition.entered.push(super::EnteredRow {
                observation_id: observation_id.to_string(),
                row: row.clone(),
            });
            recent_rows.add(observation_id, super::relay::relay_event(&row));
        }
        match self.rows.get_mut(&id) {
            Some(owned) => {
                owned.observations.insert(observation_id.to_string());
                owned.row = row;
            }
            None => {
                transition.added.push(row.clone());
                self.rows.insert(
                    id,
                    OwnedRow {
                        row,
                        observations: BTreeSet::from([observation_id.to_string()]),
                    },
                );
            }
        }
    }

    fn detach(
        &mut self,
        observation_id: &str,
        id: EventId,
        transition: &mut super::RowTransition,
        recent_rows: &crate::recent_rows::RecentRows,
    ) {
        if let Some(view) = self.observations.get_mut(observation_id) {
            view.ids.remove(&id);
        }
        self.detach_owner_only(observation_id, id, transition, recent_rows);
    }

    fn detach_owner_only(
        &mut self,
        observation_id: &str,
        id: EventId,
        transition: &mut super::RowTransition,
        recent_rows: &crate::recent_rows::RecentRows,
    ) {
        let departed = self.rows.get_mut(&id).and_then(|owned| {
            owned
                .observations
                .remove(observation_id)
                .then(|| (owned.row.clone(), owned.observations.is_empty()))
        });
        let Some((row, remove)) = departed else {
            return;
        };
        recent_rows.remove(observation_id, &id.to_hex());
        transition.removed.push(super::DepartedRow {
            observation_id: observation_id.to_string(),
            row: row.clone(),
        });
        if remove {
            self.rows.remove(&id);
        }
    }

    fn rows_for_ids(&self, ids: Option<&BTreeSet<EventId>>) -> Vec<ViewRow> {
        ids.into_iter()
            .flatten()
            .filter_map(|id| self.row(id))
            .collect()
    }
}
