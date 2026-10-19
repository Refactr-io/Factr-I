//! Python cron output joins the same canonical-chat delivery path as the Desktop courier.
use super::*;

pub(crate) async fn deliver(
    config: Arc<Config>, hub: Arc<Hub>, observer: Arc<Observer>, p: &Value,
) -> std::result::Result<Value, Value> {
    let (to_ws, mut rx) = mpsc::channel::<Message>(1024);
    let client = Arc::new(Client { id: hub.next_client_id(), to_ws: to_ws.clone(), sessions: Mutex::new(Default::default()) });
    let conn = Conn::new(config, to_ws, hub, client, observer, false, "bot", None, None);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let result: Result<Value, RpcError> = async {
        conn.ensure_control().await.map_err(RpcError::internal)?;
        let mut reply = conn.deliver_rpc(p, None).await?;
        while reply["status"] == "pending" {
            reply = conn.bot_deliver("bot.chat.deliver_status", &json!({"envelope_id":reply["envelope_id"]})).await?;
        }
        Ok(reply)
    }.await;
    end_run(&conn, None).await;
    drain.abort();
    result.map_err(|error| json!({"ok":false,"error":error.message,"data":error.data}))
}
