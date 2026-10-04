use factr_swarm_core::ChannelIndex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

type ChannelSubscriptions = Arc<RwLock<HashMap<String, HashMap<String, HashSet<String>>>>>;

async fn with_channel_index_mut(
    channel_subscriptions: &ChannelSubscriptions,
    channel_subscriptions_by_session: &ChannelSubscriptions,
    mutate: impl FnOnce(&mut ChannelIndex),
) {
    let mut subs = channel_subscriptions.write().await;
    let mut reverse = channel_subscriptions_by_session.write().await;
    let mut index = ChannelIndex {
        by_swarm_channel: std::mem::take(&mut *subs),
        by_session: std::mem::take(&mut *reverse),
    };
    mutate(&mut index);
    *subs = index.by_swarm_channel;
    *reverse = index.by_session;
}

pub(super) async fn remove_session_channel_subscriptions(
    session_id: &str,
    channel_subscriptions: &ChannelSubscriptions,
    channel_subscriptions_by_session: &ChannelSubscriptions,
) {
    with_channel_index_mut(
        channel_subscriptions,
        channel_subscriptions_by_session,
        |index| index.remove_session(session_id),
    )
    .await;
}

pub(super) async fn subscribe_session_to_channel(
    session_id: &str,
    swarm_id: &str,
    channel: &str,
    channel_subscriptions: &ChannelSubscriptions,
    channel_subscriptions_by_session: &ChannelSubscriptions,
) {
    with_channel_index_mut(
        channel_subscriptions,
        channel_subscriptions_by_session,
        |index| index.subscribe(session_id, swarm_id, channel),
    )
    .await;
}

pub(super) async fn unsubscribe_session_from_channel(
    session_id: &str,
    swarm_id: &str,
    channel: &str,
    channel_subscriptions: &ChannelSubscriptions,
    channel_subscriptions_by_session: &ChannelSubscriptions,
) {
    with_channel_index_mut(
        channel_subscriptions,
        channel_subscriptions_by_session,
        |index| index.unsubscribe(session_id, swarm_id, channel),
    )
    .await;
}
