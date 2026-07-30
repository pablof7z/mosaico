//! Executable Gherkin acceptance suite for the real Mosaico binary.

mod steps;
mod world;

use std::path::PathBuf;

use cucumber::World as _;

use world::MosaicoWorld;

fn validate_tags(tags: &[String]) {
    let excluded = tags.iter().any(|tag| tag == "wip" || tag == "designed");
    if excluded {
        assert!(
            tags.iter().any(|tag| tag.starts_with("issue-")),
            "@wip and @designed scenarios must carry an @issue-N tag"
        );
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let features = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/features"));
    let run_live = std::env::var("MOSAICO_BDD_LIVE").as_deref() == Ok("1");
    let only_live = std::env::var("MOSAICO_BDD_ONLY_LIVE").as_deref() == Ok("1");

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
        .filter_run_and_exit(features, move |_feature, _rule, scenario| {
            validate_tags(&scenario.tags);
            let live = scenario.tags.iter().any(|tag| tag == "live");
            let excluded = scenario
                .tags
                .iter()
                .any(|tag| tag == "wip" || tag == "designed");
            !excluded && if only_live { live } else { !live || run_live }
        })
        .await;
}
