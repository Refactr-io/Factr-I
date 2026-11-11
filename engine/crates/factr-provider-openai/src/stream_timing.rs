//! Optional per-call stream timing for the OpenAI Responses stream (`FACTR_STREAM_TIMING=1`).
//! Times are application-observed: `t0` is taken just before the request is handed to the
//! transport, and each event is stamped when the runtime read its frame or chunk.

use factr_provider_core::request_shape::RequestShape;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Only the exact value `1` turns the timing line on.
pub fn resolve(env: &dyn Fn(&str) -> Option<String>) -> bool {
    env("FACTR_STREAM_TIMING").as_deref() == Some("1")
}

fn print_line(line: &str) {
    eprintln!("{line}");
}

/// Where finished lines go: stderr when the flag is on, `None` (no timing at all) when it is off.
/// The flag is read once per process.
pub fn sink() -> Option<fn(&str)> {
    static ON: OnceLock<bool> = OnceLock::new();
    ON.get_or_init(|| resolve(&|name| std::env::var(name).ok()))
        .then_some(print_line as fn(&str))
}

/// Where one model call is before any of its attempts has its own [`StreamTiming`]. A call dropped
/// there (by a deadline or a stopped turn) prints one unfinished line with `phase: before_attempt`
/// and this `stage`; once an attempt starts, or the call ends, it prints nothing.
pub struct CallStage {
    sink: fn(&str),
    begun: Instant,
    stage: &'static str,
}

impl CallStage {
    pub fn start(sink: Option<fn(&str)>) -> Option<Self> {
        Some(Self { sink: sink?, begun: Instant::now(), stage: "build" })
    }

    /// `attempt` (the attempt reports itself) and `done` print nothing when dropped.
    pub fn set(stage: &mut Option<Self>, name: &'static str) {
        if let Some(stage) = stage {
            stage.stage = name;
        }
    }
}

impl Drop for CallStage {
    fn drop(&mut self) {
        if !matches!(self.stage, "attempt" | "done") {
            let line = serde_json::json!({
                "factr_stream_unfinished": 1,
                "transport": "none",
                "phase": "before_attempt",
                "stage": self.stage,
                "elapsed_ms": self.begun.elapsed().as_millis() as u64,
            });
            (self.sink)(&line.to_string());
        }
    }
}

/// Whether the consumer of the call has gone (the call was cancelled, for example by a deadline).
pub type CancelProbe = std::sync::Arc<dyn Fn() -> bool + Send + Sync>;

/// Timing for one model call attempt. Emits one JSON line when dropped: the timing line if the attempt
/// reached a terminal response event, otherwise an unfinished-attempt line naming the phase it ended in.
pub struct StreamTiming {
    sink: fn(&str),
    begun: Instant,
    t0: Instant,
    sent: bool,
    cancelled: Option<CancelProbe>,
    transport: &'static str,
    request_fields: Value,
    first_event_ms: Option<u64>,
    first_output_ms: Option<u64>,
    last_event_ms: Option<u64>,
    channel_wait: Duration,
    terminal: Option<Value>,
}

impl StreamTiming {
    /// `None` when `sink` is `None`, so nothing is observed or printed with the flag off.
    pub fn start(
        sink: Option<fn(&str)>,
        transport: &'static str,
        request: &Value,
        shape: &RequestShape,
    ) -> Option<Self> {
        let sink = sink?;
        let now = Instant::now();
        Some(Self {
            sink,
            begun: now,
            t0: now,
            sent: true,
            cancelled: None,
            transport,
            request_fields: serde_json::json!({
                "strict_tools": shape.strict_tools,
                "parallel_tool_calls": request.get("parallel_tool_calls").and_then(Value::as_bool).unwrap_or(false),
                "cache_key_sent": request.get("prompt_cache_key").is_some(),
                "native_compaction_sent": request.get("context_management").is_some(),
                "reasoning_id_replayed": shape.replay_reasoning_id,
            }),
            first_event_ms: None,
            first_output_ms: None,
            last_event_ms: None,
            channel_wait: Duration::ZERO,
            terminal: None,
        })
    }

    /// Sees one raw event (before conversion), stamped with the time its frame or chunk was read.
    pub fn observe(&mut self, data: &str, read_at: Instant) {
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            return;
        };
        let ms = read_at.saturating_duration_since(self.t0).as_millis() as u64;
        self.first_event_ms.get_or_insert(ms);
        self.last_event_ms = Some(ms);
        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "response.output_text.delta"
            | "response.refusal.delta"
            | "response.function_call_arguments.delta"
            | "response.custom_tool_call_input.delta"
            | "response.reasoning.delta"
            | "response.reasoning_summary_text.delta" => {
                self.first_output_ms.get_or_insert(ms);
            }
            "response.completed" | "response.incomplete" | "response.failed" => {
                self.terminal = Some(event.get("response").cloned().unwrap_or(Value::Null));
            }
            _ => {}
        }
    }

    /// The request is not sent yet: [`Self::sending`] restarts `t0` just before it goes out and
    /// [`Self::sent`] marks it gone. An attempt dropped before that is reported as `not_sent`.
    pub fn not_sent_yet(&mut self) {
        self.sent = false;
    }

    pub fn sending(&mut self) {
        self.t0 = Instant::now();
    }

    pub fn sent(&mut self) {
        self.sent = true;
    }

    /// An attempt dropped while `probe` says the consumer is gone is reported as `cancelled`.
    pub fn watch(&mut self, probe: CancelProbe) {
        self.cancelled = Some(probe);
    }

    /// Where an attempt with no terminal response ended.
    fn phase(&self) -> &'static str {
        if self.cancelled.as_ref().is_some_and(|gone| gone()) {
            "cancelled"
        } else if !self.sent {
            "not_sent"
        } else if self.first_event_ms.is_none() {
            "no_first_event"
        } else {
            "cut_mid_stream"
        }
    }

    /// `factr_stream_unfinished` (not `factr_stream_timing`, so call counts that grep for the timing
    /// line are unchanged).
    fn unfinished_line(&self) -> String {
        let mut line = serde_json::json!({
            "factr_stream_unfinished": 1,
            "transport": self.transport,
            "phase": self.phase(),
            "elapsed_ms": self.begun.elapsed().as_millis() as u64,
            "first_event_ms": self.first_event_ms,
            "last_event_ms": self.last_event_ms,
        });
        if let (Some(line), Some(fields)) = (line.as_object_mut(), self.request_fields.as_object()) {
            line.extend(fields.clone());
        }
        line.to_string()
    }

    pub fn add_channel_wait(&mut self, waited: Duration) {
        self.channel_wait += waited;
    }

    fn line(&self, response: &Value) -> String {
        let usage = &response["usage"];
        let mut line = serde_json::json!({
            "factr_stream_timing": 1,
            "transport": self.transport,
            "first_event_ms": self.first_event_ms,
            "first_output_ms": self.first_output_ms,
            "last_event_ms": self.last_event_ms,
            "channel_wait_ms": self.channel_wait.as_millis() as u64,
            "input_tokens": usage["input_tokens"].as_u64(),
            "cached_tokens": usage["input_tokens_details"]["cached_tokens"].as_u64(),
            "output_tokens": usage["output_tokens"].as_u64(),
            "reasoning_tokens": usage["output_tokens_details"]["reasoning_tokens"].as_u64(),
            "served_service_tier": response["service_tier"].as_str(),
            "response_model": response["model"].as_str(),
            // A failed or incomplete response says why (codes only, never message text).
            "status": response["status"].as_str(),
            "error_code": response["error"]["code"].as_str(),
            "incomplete_reason": response["incomplete_details"]["reason"].as_str(),
        });
        if let (Some(line), Some(fields)) = (line.as_object_mut(), self.request_fields.as_object())
        {
            line.extend(fields.clone());
        }
        line.to_string()
    }
}

impl Drop for StreamTiming {
    fn drop(&mut self) {
        match &self.terminal {
            Some(response) => (self.sink)(&self.line(response)),
            None => (self.sink)(&self.unfinished_line()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_is_on_only_for_exactly_one() {
        assert!(!resolve(&|_| None));
        for junk in ["0", "true", "yes", " 1", ""] {
            assert!(!resolve(&|_| Some(junk.to_string())), "{junk:?}");
        }
        assert!(resolve(
            &|name| (name == "FACTR_STREAM_TIMING").then(|| "1".to_string())
        ));
    }

    #[test]
    fn no_sink_means_no_timing() {
        let request = serde_json::json!({});
        assert!(StreamTiming::start(None, "https", &request, &RequestShape::VARIANT).is_none());
    }

    static SSE_LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    /// A synthetic SSE stream through the production parser: each chunk is stamped when the
    /// stream reads it, and the terminal response fills the token, tier and model fields.
    #[test]
    fn sse_stream_emits_one_line_with_the_observed_fields() {
        use crate::stream::OpenAIResponsesStream;
        use futures::{FutureExt, StreamExt, channel::mpsc};

        let request = serde_json::json!({"parallel_tool_calls": false, "prompt_cache_key": "k"});
        let timing = StreamTiming::start(
            Some(|line| SSE_LINES.lock().unwrap().push(line.to_string())),
            "https",
            &request,
            &RequestShape::VARIANT,
        );
        let (tx, rx) = mpsc::unbounded::<Result<bytes::Bytes, reqwest::Error>>();
        let mut stream = OpenAIResponsesStream::new_with_timing(rx, timing);
        let frames = [
            r#"{"type":"response.created","response":{"id":"resp_t"}}"#,
            r#"{"type":"response.output_text.delta","delta":"hi"}"#,
            r#"{"type":"response.completed","response":{"id":"resp_t","status":"completed","model":"gpt-5.4-served","service_tier":"priority","output":[],"usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":40},"output_tokens":50,"output_tokens_details":{"reasoning_tokens":20}}}}"#,
        ];
        for frame in frames {
            std::thread::sleep(Duration::from_millis(30));
            tx.unbounded_send(Ok(bytes::Bytes::from(format!("data: {frame}\n\n"))))
                .unwrap();
            while let Some(Some(event)) = stream.next().now_or_never() {
                event.unwrap();
            }
        }
        assert!(stream.timing_mut().is_some());
        drop(stream);

        let lines = SSE_LINES.lock().unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        let line: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(line["factr_stream_timing"], 1);
        assert_eq!(line["transport"], "https");
        let first = line["first_event_ms"].as_u64().unwrap();
        let output = line["first_output_ms"].as_u64().unwrap();
        let last = line["last_event_ms"].as_u64().unwrap();
        assert!(first >= 30 && first < output && output < last, "{line}");
        assert_eq!(line["input_tokens"], 100);
        assert_eq!(line["cached_tokens"], 40);
        assert_eq!(line["output_tokens"], 50);
        assert_eq!(line["reasoning_tokens"], 20);
        assert_eq!(line["served_service_tier"], "priority");
        assert_eq!(line["response_model"], "gpt-5.4-served");
        assert_eq!(line["channel_wait_ms"], 0);
        assert_eq!(line["strict_tools"], false);
        assert_eq!(line["parallel_tool_calls"], false);
        assert_eq!(line["cache_key_sent"], true);
        assert_eq!(line["native_compaction_sent"], false);
        assert_eq!(line["reasoning_id_replayed"], true);
        assert_eq!((line["status"].as_str(), line["error_code"].is_null()), (Some("completed"), true));
    }

    static UNFINISHED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    /// An attempt dropped before its terminal response prints one unfinished line naming where it ended.
    #[test]
    fn an_attempt_with_no_terminal_response_names_its_phase() {
        let request = serde_json::json!({});
        let start = || StreamTiming::start(Some(|line| UNFINISHED.lock().unwrap().push(line.to_string())), "websocket", &request, &RequestShape::VARIANT).unwrap();
        let mut not_sent = start();
        not_sent.not_sent_yet();
        drop(not_sent);
        drop(start());
        let mut cut = start();
        cut.observe(r#"{"type":"response.created"}"#, Instant::now());
        drop(cut);
        let mut cancelled = start();
        cancelled.observe(r#"{"type":"response.created"}"#, Instant::now());
        cancelled.watch(std::sync::Arc::new(|| true));
        drop(cancelled);
        let mut done = start();
        done.observe(r#"{"type":"response.completed","response":{}}"#, Instant::now());
        drop(done);
        let lines: Vec<Value> = UNFINISHED.lock().unwrap().iter().map(|l| serde_json::from_str(l).unwrap()).collect();
        let phases: Vec<_> = lines.iter().filter_map(|l| l["phase"].as_str()).collect();
        assert_eq!(phases, ["not_sent", "no_first_event", "cut_mid_stream", "cancelled"]);
        assert!(lines[..4].iter().all(|l| l["factr_stream_unfinished"] == 1 && l.get("factr_stream_timing").is_none() && l["elapsed_ms"].is_u64()));
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[4]["factr_stream_timing"], 1, "a finished attempt prints the timing line only");
    }

    static STAGES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    #[test]
    fn a_call_dropped_before_any_attempt_names_its_stage() {
        let sink: Option<fn(&str)> = Some(|line| STAGES.lock().unwrap().push(line.to_string()));
        let mut waiting = CallStage::start(sink);
        CallStage::set(&mut waiting, "cooldown");
        drop(waiting);
        for quiet in ["attempt", "done"] {
            let mut call = CallStage::start(sink);
            CallStage::set(&mut call, quiet);
            drop(call);
        }
        assert!(CallStage::start(None).is_none());
        let lines = STAGES.lock().unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        let line: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!((line["phase"].as_str(), line["stage"].as_str()), (Some("before_attempt"), Some("cooldown")));
    }

    #[test]
    fn existing_sse_callers_get_no_observer() {
        let (_tx, rx) = futures::channel::mpsc::unbounded::<Result<bytes::Bytes, reqwest::Error>>();
        let mut stream = crate::stream::OpenAIResponsesStream::new(rx);
        assert!(stream.timing_mut().is_none());
    }
}
