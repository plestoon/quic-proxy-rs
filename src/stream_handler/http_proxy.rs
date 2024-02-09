use anyhow::{anyhow, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use url::{Position, Url};

use crate::stream_handler::StreamHandler;
use crate::transport::Transport;

#[derive(Debug, Clone)]
pub struct HttpProxy {
    auth_config: Option<AuthConfig>,
}

impl HttpProxy {
    pub fn new(auth_config: Option<AuthConfig>) -> HttpProxy {
        HttpProxy {
            auth_config
        }
    }
}

impl StreamHandler for HttpProxy {
    async fn handle_stream(&self, inbound: impl Transport) -> Result<()> {
        let (mut inbound, request) = parse_request(inbound).await?;

        if !authenticate(self.auth_config.as_ref(), &request) {
            send_407(&mut inbound).await?;

            return Ok(());
        }

        match request.request_line.method.as_str() {
            "CONNECT" => {
                handle_https(inbound, &request).await?;
            }
            _ => {
                handle_http(inbound, &request).await?;
            }
        }

        Ok(())
    }
}

fn authenticate(auth_config: Option<&AuthConfig>, request: &HttpRequest) -> bool {
    if let (Some(auth_config), Some((username, password))) = (auth_config, request.auth.as_ref()) {
        auth_config.verify(username, password)
    } else {
        auth_config.is_none()
    }
}

async fn send_407(inbound: &mut impl Transport) -> Result<()> {
    let response_line = "HTTP/1.1 407 Proxy Authentication Required";
    let headers = "Proxy-Authenticate: Basic realm=\"QUIC Proxy\"";
    let response = format!("{}\r\n{}\r\n\r\n", response_line, headers);

    inbound
        .write_all(response.as_bytes())
        .await?;

    Ok(())
}

async fn handle_http(
    mut inbound: BufReader<impl Transport>,
    request: &HttpRequest,
) -> Result<()> {
    let url = Url::parse(request.request_line.uri.as_str())?;
    let host = url.host_str().unwrap();
    let port = url.port().unwrap_or(80);
    let path = &url[Position::BeforePath..];
    let request_line = format!(
        "{} {} {}\r\n",
        request.request_line.method, path, request.request_line.version
    );
    let addr = format!("{}:{}", host, port);
    let mut outbound = TcpStream::connect(addr).await?;
    outbound.write_all(request_line.as_bytes()).await?;
    outbound.write_all(serialize_headers(&request.headers).as_bytes()).await?;
    tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await?;

    Ok(())
}

async fn handle_https(mut inbound: BufReader<impl Transport>, request: &HttpRequest) -> Result<()> {
    let mut outbound = TcpStream::connect(&request.request_line.uri).await?;
    inbound
        .write_all("HTTP/1.1 200 Connection Established\r\n\r\n".as_bytes())
        .await?;
    tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await?;

    Ok(())
}

async fn parse_request(inbound: impl Transport) -> Result<(BufReader<impl Transport>, HttpRequest)> {
    let reader = BufReader::new(inbound);
    let mut lines = reader.lines();
    let request_line = lines.next_line().await?.ok_or(anyhow!("failed to read request line"))?;
    let request_line = HttpRequestLine::parse(request_line);

    let mut auth: Option<(String, String)> = None;
    let mut headers = Vec::<(String, String)>::new();

    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            break;
        }

        let (key, value) = line.split_once(": ").unwrap();
        if key.to_lowercase() == "proxy-authorization" {
            let (_, credentials) = value.split_once(' ').unwrap();
            let credentials = String::from_utf8(URL_SAFE.decode(credentials.as_bytes())?)?;
            let (username, password) = credentials.split_once(':').unwrap();
            auth = Some((username.into(), password.into()));
        } else {
            headers.push((key.into(), value.into()));
        }
    }

    let request = HttpRequest {
        request_line,
        auth,
        headers,
    };

    Ok((lines.into_inner(), request))
}

fn serialize_headers(headers: &Vec<(String, String)>) -> String {
    let headers = headers.iter().map(|(key, value)| format!("{}: {}", key, value)).collect::<Vec<String>>();
    let headers = headers.join("\r\n");

    format!("{}\r\n\r\n", headers)
}

struct HttpRequestLine {
    method: String,
    uri: String,
    version: String,
}

impl HttpRequestLine {
    fn parse(line: String) -> Self {
        let parts = line.split(' ').collect::<Vec<&str>>();

        HttpRequestLine {
            method: parts[0].to_string(),
            uri: parts[1].to_string(),
            version: parts[2].to_string(),
        }
    }
}

struct HttpRequest {
    request_line: HttpRequestLine,
    auth: Option<(String, String)>,
    headers: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    username: String,
    password: String,
}

impl AuthConfig {
    pub fn new(username: &str, password: &str) -> Self {
        AuthConfig {
            username: username.into(),
            password: password.into(),
        }
    }

    pub fn verify(&self, username: &str, password: &str) -> bool {
        username == self.username && password == self.password
    }
}
