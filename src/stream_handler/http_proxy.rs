use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use url::{Position, Url};

use crate::stream_handler::StreamHandler;
use crate::transport::Transport;

#[derive(Debug, Clone)]
pub struct HttpProxy {}

impl HttpProxy {
    pub fn new() -> HttpProxy {
        HttpProxy {}
    }
}

impl Default for HttpProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamHandler for HttpProxy {
    async fn handle_stream(&self, stream: impl Transport) -> anyhow::Result<()> {
        let stream = BufReader::new(stream);
        let mut lines = stream.lines();
        if let Some(request_line) = lines.next_line().await? {
            println!("{}", request_line);
            let request_line = HttpRequestLine::parse(request_line);
            let mut socket = lines.into_inner();
            match request_line.method.as_str() {
                "CONNECT" => {
                    handle_https(socket, request_line.uri).await?;
                }
                _ => {
                    handle_http(&mut socket, request_line).await?;
                }
            }
        }

        Ok(())
    }
}

async fn handle_http(
    socket: &mut BufReader<impl Transport>,
    request_line: HttpRequestLine,
) -> anyhow::Result<()> {
    let url = Url::parse(request_line.uri.as_str())?;
    let domain = url.host_str().unwrap();
    let port = url.port().unwrap_or(80);
    let path = &url[Position::BeforePath..];
    let request_line = format!(
        "{} {} {}\r\n",
        request_line.method, path, request_line.version
    );
    let host = format!("{}:{}", domain, port);
    let mut remote = TcpStream::connect(host).await?;
    remote.write_all(request_line.as_bytes()).await?;
    tokio::io::copy_bidirectional(socket, &mut remote).await?;

    Ok(())
}

async fn handle_https(socket: BufReader<impl Transport>, host: String) -> anyhow::Result<()> {
    let mut lines = socket.lines();
    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            let mut socket = lines.into_inner();
            socket
                .write_all("HTTP/1.1 200 Connection Established\r\n\r\n".as_bytes())
                .await?;
            let mut remote = TcpStream::connect(host).await?;
            tokio::io::copy_bidirectional(&mut socket, &mut remote).await?;
            break;
        }
    }

    Ok(())
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
