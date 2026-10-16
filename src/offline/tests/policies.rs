use super::{fixtures, group};

#[tokio::test(flavor = "current_thread")]
async fn reusable_policies_compose_reject_and_commit_observations() {
    group(&fixtures("policies")).await;
}
