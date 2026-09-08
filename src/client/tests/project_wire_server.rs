use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinHandle,
};
use tokio_tungstenite::tungstenite::Message;
use url::Url;

pub(super) const KEY: &str = "local-fake-guide-key-ABC123";

#[derive(Clone, Debug)]
pub(super) struct HttpCapture {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub(super) struct Fixture {
    pub address: SocketAddr,
    pub http: Arc<Mutex<Vec<HttpCapture>>>,
    pub wire: mpsc::Receiver<Value>,
    task: JoinHandle<()>,
}

impl Fixture {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let http = Arc::new(Mutex::new(Vec::new()));
        let captured = http.clone();
        let (sender, wire) = mpsc::channel(4);
        let task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let captured = captured.clone();
                let sender = sender.clone();
                tokio::spawn(async move {
                    serve_connection(stream, address, captured, sender).await;
                });
            }
        });
        Self {
            address,
            http,
            wire,
            task,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_connection(
    mut stream: TcpStream,
    address: SocketAddr,
    captured: Arc<Mutex<Vec<HttpCapture>>>,
    sender: mpsc::Sender<Value>,
) {
    let mut preview = [0_u8; 16384];
    let header = loop {
        let length = stream.peek(&mut preview).await.unwrap();
        if length == 0 {
            return;
        }
        if let Some(end) = preview[..length].windows(4).position(|v| v == b"\r\n\r\n") {
            break String::from_utf8(preview[..end + 4].to_vec()).unwrap();
        }
        assert!(length < preview.len(), "bounded local fixture headers");
        tokio::task::yield_now().await;
    };
    let headers: BTreeMap<_, _> = header
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    if headers
        .get("upgrade")
        .is_some_and(|value| value == "websocket")
    {
        assert_eq!(headers.get("api-key").map(String::as_str), Some(KEY));
        serve_socket(stream, sender).await;
        return;
    }
    let first = header
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .collect::<Vec<_>>();
    let url = Url::parse(&format!("http://{address}{}", first[1])).unwrap();
    let body_length = headers
        .get("content-length")
        .map_or(0, |value| value.parse::<usize>().unwrap());
    assert!(body_length < 8 * 1024 * 1024);
    let mut body = vec![0_u8; header.len() + body_length];
    stream.read_exact(&mut body).await.unwrap();
    body.drain(..header.len());
    let request = HttpCapture {
        method: first[0].into(),
        path: url.path().into(),
        query: url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect(),
        headers,
        body,
    };
    let response = response(&request, address);
    captured.lock().push(request);
    let body = response.to_string();
    stream.write_all(format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ).as_bytes()).await.unwrap();
}

fn response(request: &HttpCapture, address: SocketAddr) -> Value {
    if request.path == "/fixture-upload" {
        assert_eq!(request.method, "PUT");
        assert!(!request.headers.contains_key("api-key"));
        assert!(!request.headers.contains_key("authorization"));
        assert!(!request.headers.contains_key("cookie"));
        return json!({});
    }
    assert_eq!(
        request.headers.get("api-key").map(String::as_str),
        Some(KEY)
    );
    match request.path.as_str() {
        "/v1/account/me" => {
            json!({"status":"success","data":{"username":"fixture","walletAddress":"fixture"}})
        }
        "/api/v1/models/list" => json!([
            {"id":"flux1-schnell-fp8","SID":1,"tier":"fixture"},
            {"id":"z_image_turbo_bf16","SID":2,"tier":"comfy-fixture"},
            {"id":"sam3_image_segment_bf16","SID":3,"tier":"utility","media":"image"},
            {"id":"pixal3d_int8_i23d","SID":4,"tier":"utility","media":"model"}
        ]),
        "/api/v2/models/tiers" => json!({"fixture": {
            "steps":{"min":1,"max":5,"default":4},
            "guidance":{"min":1,"max":1,"default":1},
            "sampler":{"allowed":["Euler"],"default":"Euler"},
            "scheduler":{"allowed":["Simple","DDIM"],"default":"Simple"}
        }, "utility": {
            "type":"image", "steps":{"min":1,"max":56,"default":1},
            "guidance":{"min":0,"max":1,"default":0},
            "sampler":{"allowed":[],"default":null},
            "scheduler":{"allowed":[],"default":null}
        }, "comfy-fixture": {
            "type":"image", "steps":{"min":4,"max":12,"default":8},
            "guidance":{"min":1,"max":1,"default":1},
            "comfySampler":{"allowed":["res_multistep","Euler"],"default":"res_multistep"},
            "comfyScheduler":{"allowed":["Simple"],"default":"Simple"}
        }}),
        "/api/v1/artist/projects/sync" => {
            json!({"activeProjects":[],"unclaimedCompletedProjects":[]})
        }
        "/v1/image/uploadUrl" => {
            json!({"status":"success","data":{"uploadUrl":format!("http://{address}/fixture-upload")}})
        }
        other => panic!("unexpected local fixture path {other}"),
    }
}

async fn serve_socket(stream: TcpStream, sender: mpsc::Sender<Value>) {
    let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
    let payload = crate::utils::b64_json_encode(&json!({
        "username":"fixture", "address":"fixture", "subscriptionEntitlement":{}
    }))
    .unwrap();
    socket
        .send(Message::Text(
            json!({"type":"authenticated","data":payload})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    while let Some(Ok(message)) = socket.next().await {
        match message {
            Message::Text(text) => {
                let envelope: Value = serde_json::from_str(&text).unwrap();
                if envelope["type"] == "jobRequest" {
                    let value =
                        crate::utils::b64_json_decode(envelope["data"].as_str().unwrap()).unwrap();
                    sender.send(value.clone()).await.unwrap();
                    if matches!(
                        value["keyFrames"][0]["modelID"].as_str(),
                        Some("sam3_image_segment_bf16" | "pixal3d_int8_i23d")
                    ) {
                        let project_id = &value["jobID"];
                        for (name, data) in [
                            (
                                "jobResult",
                                json!({"jobID":project_id,"imgID":"UTILITY-RESULT","resultUrl":"https://example.test/utility-result"}),
                            ),
                            (
                                "jobState",
                                json!({"jobID":project_id,"type":"jobCompleted"}),
                            ),
                        ] {
                            socket.send(Message::Text(json!({
                                "type":name,"data":crate::utils::b64_json_encode(&data).unwrap()
                            }).to_string().into())).await.unwrap();
                        }
                    }
                }
            }
            Message::Ping(value) => {
                let _ = socket.send(Message::Pong(value)).await;
            }
            _ => {}
        }
    }
}
