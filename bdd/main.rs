//! Executable Gherkin acceptance suite for the real Mosaico binary.

mod steps;
mod world;

use std::path::PathBuf;

use cucumber::World as _;

use world::MosaicoWorld;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let features = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/features"));

    MosaicoWorld::cucumber()
        .max_concurrent_scenarios(1)
        .after(|_feature, _rule, scenario, finished, world| {
            Box::pin(async move {
                match finished {
                    cucumber::event::ScenarioFinished::StepPassed => {
                        MosaicoWorld::remove_failure_artifacts(&scenario.name);
                    }
                    _ => {
                        if let Some(world) = world {
                            world.retain_failure_artifacts(&scenario.name);
                        }
                    }
                }
            })
        })
        .run_and_exit(features)
        .await;
}
