use std::collections::HashMap;

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
        let (mut inbound, request_line, mut headers) = parse_request(inbound).await?;

        if self.auth_config.is_some() && !authenticate(&self.auth_config.clone().unwrap(), &mut headers).await? {
            send_407_and_close_transport(&mut inbound).await?;
            return Ok(());
        }

        match request_line.method.as_str() {
            "CONNECT" => {
                handle_https(inbound, request_line).await?;
            }
            _ => {
                handle_http(inbound, request_line, headers).await?;
            }
        }

        Ok(())
    }
}

async fn authenticate(auth_config: &AuthConfig, headers: &mut HashMap<String, String>) -> Result<bool> {
    let proxy_authorization_header = headers.remove("Proxy-Authorization");
    if let Some(proxy_authorization_header) = proxy_authorization_header {
        Ok(auth_config.verify(&proxy_authorization_header)?)
    } else {
        Ok(false)
    }
}

async fn send_407_and_close_transport(inbound: &mut impl Transport) -> Result<()> {
    inbound
        .write_all("HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"QUIC Proxy\"\r\n\r\n".as_bytes())
        .await?;
    inbound.shutdown().await?;

    Ok(())
}

async fn handle_http(
    mut inbound: BufReader<impl Transport>,
    request_line: HttpRequestLine,
    headers: HashMap<String, String>,
) -> Result<()> {
    let url = Url::parse(request_line.uri.as_str())?;
    let domain = url.host_str().unwrap();
    let port = url.port().unwrap_or(80);
    let path = &url[Position::BeforePath..];
    let request_line = format!(
        "{} {} {}\r\n",
        request_line.method, path, request_line.version
    );
    let host = format!("{}:{}", domain, port);
    let mut outbound = TcpStream::connect(host).await?;
    outbound.write_all(request_line.as_bytes()).await?;
    outbound.write_all(serialize_headers(&headers).as_bytes()).await?;
    tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await?;

    Ok(())
}

async fn handle_https(mut inbound: BufReader<impl Transport>, request_line: HttpRequestLine) -> Result<()> {
    let mut outbound = TcpStream::connect(request_line.uri).await?;
    inbound
        .write_all("HTTP/1.1 200 Connection Established\r\n\r\n".as_bytes())
        .await?;
    tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await?;

    Ok(())
}

async fn parse_request(inbound: impl Transport) -> Result<(BufReader<impl Transport>, HttpRequestLine, HashMap<String, String>)> {
    let reader = BufReader::new(inbound);
    let mut lines = reader.lines();
    let request_line = lines.next_line().await?.ok_or(anyhow!("failed to read request line"))?;
    let request_line = HttpRequestLine::parse(request_line);

    let mut headers = HashMap::<String, String>::new();
    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            break;
        }

        let parts = line.split(": ").collect::<Vec<&str>>();
        headers.insert(parts[0].to_string(), parts[1].to_string());
    }

    Ok((lines.into_inner(), request_line, headers))
}

fn serialize_headers(headers: &HashMap<String, String>) -> String {
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

    pub fn verify(&self, proxy_authorization_header: &str) -> Result<bool> {
        let (_, credentials) = proxy_authorization_header.split_once(' ').unwrap();
        let credentials = String::from_utf8(URL_SAFE.decode(credentials.as_bytes())?)?;
        let (username, password) = credentials.split_once(':').unwrap();

        Ok(username == self.username && password == self.password)
    }
}
