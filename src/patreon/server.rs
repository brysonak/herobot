use super::user;
use hmac::{Hmac, Mac};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use md5::Md5;
use poise::serenity_prelude as serenity;
use serde_json::Value;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::net::TcpListener;

const ADDR: &str = "127.0.0.1:8080";
const PATH: &str = "/patreon/webhook";
const EVENT: &str = "members:pledge:create";
const MAX_BODY: usize = 256 * 1024;

pub struct Hook {
    pub http: Arc<serenity::Http>,
    pub guild: serenity::GuildId,
    pub log_channel: serenity::ChannelId,
    pub secret: Vec<u8>,
    pub db: crate::spam::Db,
}

type Resp = Response<Full<Bytes>>;

fn status(code: StatusCode) -> Resp {
    let mut r = Response::new(Full::new(Bytes::new()));
    *r.status_mut() = code;
    r
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn verified(secret: &[u8], body: &[u8], sig_hex: &str) -> bool {
    let Some(sig) = unhex(sig_hex) else {
        return false;
    };
    let mut mac = Hmac::<Md5>::new_from_slice(secret).expect("hmac takes any key length");
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

async fn handle(hook: Arc<Hook>, req: Request<Incoming>) -> Result<Resp, Infallible> {
    if req.method() != Method::POST || req.uri().path() != PATH {
        return Ok(status(StatusCode::NOT_FOUND));
    }

    let header = |name: &str| {
        req.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let sig = header("x-patreon-signature");
    let event = header("x-patreon-event");

    let body = match Limited::new(req.into_body(), MAX_BODY).collect().await {
        Ok(b) => b.to_bytes(),
        Err(_) => return Ok(status(StatusCode::PAYLOAD_TOO_LARGE)),
    };

    if !sig.is_some_and(|s| verified(&hook.secret, &body, &s)) {
        return Ok(status(StatusCode::UNAUTHORIZED));
    }

    if event.as_deref() != Some(EVENT) {
        eprintln!("patreon: ignoring event {event:?}");
        return Ok(status(StatusCode::OK));
    }

    let payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("patreon: bad json: {e}");
            return Ok(status(StatusCode::BAD_REQUEST));
        }
    };

    tokio::spawn(async move {
        if let Err(e) = user::subscribed(&hook, &payload).await {
            eprintln!("patreon: {e}");
        }
    });
    Ok(status(StatusCode::OK))
}

pub fn spawn(hook: Hook) {
    let hook = Arc::new(hook);
    tokio::spawn(async move {
        let listener = match TcpListener::bind(ADDR).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("patreon listener failed on {ADDR}: {e}");
                return;
            }
        };
        println!("patreon webhook listening on {ADDR}{PATH}");
        serve(listener, hook).await;
    });
}

async fn serve(listener: TcpListener, hook: Arc<Hook>) {
    loop {
        let stream = match listener.accept().await {
            Ok((s, _)) => s,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let hook = hook.clone();
        tokio::spawn(async move {
            let svc = service_fn(move |req| handle(hook.clone(), req));
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), svc)
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    const SECRET: &[u8] = b"test-secret";
    const GOOD: &str = "members:pledge:create";

    #[test]
    fn signature() {
        let (key, msg) = (b"Jefe", b"what do ya want for nothing?");
        assert!(verified(key, msg, "750c783e6ab0b503eaa86e310a5db738"));
        assert!(!verified(key, msg, "750c783e6ab0b503eaa86e310a5db739"));
        assert!(!verified(key, b"other", "750c783e6ab0b503eaa86e310a5db738"));
        assert!(!verified(key, msg, "not hex"));
        assert!(!verified(key, msg, ""));
    }

    fn sign(body: &[u8]) -> String {
        let mut mac = Hmac::<Md5>::new_from_slice(SECRET).unwrap();
        mac.update(body);
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    async fn start() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hook = Hook {
            http: Arc::new(serenity::Http::new("test")),
            guild: serenity::GuildId::new(1),
            log_channel: serenity::ChannelId::new(1),
            secret: SECRET.to_vec(),
            db: Arc::new(std::sync::Mutex::new(crate::db::open(":memory:").unwrap())),
        };
        tokio::spawn(serve(listener, Arc::new(hook)));
        addr
    }

    async fn send(method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> u16 {
        let mut s = TcpStream::connect(start().await).await.unwrap();
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        s.write_all(body).await.unwrap();

        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out.split_whitespace().nth(1).unwrap().parse().unwrap()
    }

    #[tokio::test]
    async fn only_the_webhook_route_exists() {
        let sig = sign(b"{}");
        let h = [
            ("x-patreon-signature", sig.as_str()),
            ("x-patreon-event", GOOD),
        ];
        assert_eq!(send("POST", "/", &h, b"{}").await, 404);
        assert_eq!(send("POST", "/patreon/callback", &h, b"{}").await, 404);
        assert_eq!(send("GET", PATH, &h, b"").await, 404);
    }

    #[tokio::test]
    async fn unsigned_or_forged_is_rejected() {
        let body = br#"{"data":{}}"#;
        let other = sign(b"something else");
        let event = ("x-patreon-event", GOOD);
        assert_eq!(send("POST", PATH, &[event], body).await, 401);
        assert_eq!(
            send("POST", PATH, &[event, ("x-patreon-signature", "")], body).await,
            401
        );
        assert_eq!(
            send("POST", PATH, &[event, ("x-patreon-signature", "zz")], body).await,
            401
        );
        assert_eq!(
            send(
                "POST",
                PATH,
                &[event, ("x-patreon-signature", &other)],
                body
            )
            .await,
            401
        );
    }

    #[tokio::test]
    async fn bad_signature_beats_bad_json() {
        let h = [("x-patreon-signature", "00"), ("x-patreon-event", GOOD)];
        assert_eq!(send("POST", PATH, &h, b"not json").await, 401);
    }

    #[tokio::test]
    async fn signed_but_other_events_are_ignored() {
        let body = br#"{"data":{}}"#;
        let sig = sign(body);
        let sig = ("x-patreon-signature", sig.as_str());
        assert_eq!(
            send(
                "POST",
                PATH,
                &[sig, ("x-patreon-event", "members:update")],
                body
            )
            .await,
            200
        );
        assert_eq!(send("POST", PATH, &[sig], body).await, 200);
    }

    #[tokio::test]
    async fn signed_garbage_for_the_right_event_is_400() {
        let body = b"not json";
        let sig = sign(body);
        let h = [
            ("x-patreon-signature", sig.as_str()),
            ("x-patreon-event", GOOD),
        ];
        assert_eq!(send("POST", PATH, &h, body).await, 400);
    }
}
