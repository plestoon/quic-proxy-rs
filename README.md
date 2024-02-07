# QUIC Proxy
An HTTP and SOCKS proxy over QUIC

## work in progress

currently, no authentication and SOCKS support yet

## Quickstart

run a local server in compatibility mode and pass traffic to a remote QUIC server

```bash
quic-proxy --listen-addr 0.0.0.0:1080 --transport tcp --passthrough-url quic://remote.host:1080
```

run a remote QUIC server

```bash
quic-proxy --listen-addr 0.0.0.0:1080 --transport quic --cert-path cert.pem --key-path key.pem
```