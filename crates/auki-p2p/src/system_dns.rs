//! DNS transport backed by the operating system resolver.
//!
//! Hickory 0.25 initializes its Unix resolver from `/etc/resolv.conf`, which
//! is not available to a physical iOS application. This adapter deliberately
//! uses Tokio's `lookup_host` instead; on iOS that reaches the public system
//! resolver through `getaddrinfo` and preserves the device's active network
//! policy instead of selecting an SDK-owned public DNS server.

use std::{
    collections::HashSet,
    error::Error as StdError,
    fmt, io,
    net::IpAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{future::BoxFuture, FutureExt};
use libp2p::{
    core::transport::{DialOpts, ListenerId, TransportError, TransportEvent},
    core::Transport,
    multiaddr::Protocol,
    Multiaddr,
};
use parking_lot::Mutex;

const MAX_DNS_LOOKUPS: usize = 32;
const MAX_RESOLVED_ADDRESSES: usize = 16;

/// A transport wrapper that resolves multiaddr DNS components with the host
/// operating system before delegating to the inner transport.
pub(crate) struct SystemDnsTransport<T> {
    inner: Arc<Mutex<T>>,
}

#[derive(Debug, thiserror::Error)]
#[error("system DNS resolution failed: {source}")]
pub(crate) struct SystemDnsError {
    #[source]
    source: io::Error,
}

impl SystemDnsError {
    pub(crate) fn new(source: io::Error) -> Self {
        Self { source }
    }
}

impl<T> SystemDnsTransport<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

impl<T> fmt::Debug for SystemDnsTransport<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SystemDnsTransport")
    }
}

impl<T> Transport for SystemDnsTransport<T>
where
    T: Transport + Send + Unpin + 'static,
    T::Error: StdError + Send + Sync + 'static,
    T::Dial: Send + 'static,
    T::ListenerUpgrade: Send + 'static,
    T::Output: Send + 'static,
{
    type Output = T::Output;
    type Error = io::Error;
    type ListenerUpgrade = BoxFuture<'static, io::Result<Self::Output>>;
    type Dial = BoxFuture<'static, io::Result<Self::Output>>;

    fn listen_on(
        &mut self,
        id: ListenerId,
        address: Multiaddr,
    ) -> Result<(), TransportError<Self::Error>> {
        self.inner
            .lock()
            .listen_on(id, address)
            .map_err(|error| error.map(io::Error::other))
    }

    fn remove_listener(&mut self, id: ListenerId) -> bool {
        self.inner.lock().remove_listener(id)
    }

    fn dial(
        &mut self,
        address: Multiaddr,
        options: DialOpts,
    ) -> Result<Self::Dial, TransportError<Self::Error>> {
        if !contains_dns_component(&address) {
            let dial = self
                .inner
                .lock()
                .dial(address, options)
                .map_err(|error| error.map(io::Error::other))?;
            return Ok(async move { dial.await.map_err(io::Error::other) }.boxed());
        }

        let inner = Arc::clone(&self.inner);
        Ok(async move {
            let addresses = resolve_multiaddr(address)
                .await
                .map_err(|error| io::Error::other(SystemDnsError::new(error)))?;
            let mut last_error = None;

            for resolved in addresses {
                let dial = {
                    let mut transport = inner.lock();
                    transport.dial(resolved.clone(), options)
                };

                let dial = match dial {
                    Ok(dial) => dial,
                    Err(TransportError::MultiaddrNotSupported(_)) => {
                        last_error = Some(io::Error::new(
                            io::ErrorKind::Unsupported,
                            format!("resolved address is unsupported: {resolved}"),
                        ));
                        continue;
                    }
                    Err(TransportError::Other(error)) => {
                        last_error = Some(io::Error::other(error));
                        continue;
                    }
                };

                match dial.await {
                    Ok(output) => return Ok(output),
                    Err(error) => last_error = Some(io::Error::other(error)),
                }
            }

            Err(last_error.unwrap_or_else(|| {
                io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    "system DNS returned no dialable addresses",
                )
            }))
        }
        .boxed())
    }

    fn poll(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        let mut inner = self.inner.lock();
        Transport::poll(Pin::new(&mut *inner), context).map(|event| {
            event
                .map_upgrade(|upgrade| {
                    async move { upgrade.await.map_err(io::Error::other) }.boxed()
                })
                .map_err(io::Error::other)
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AddressFamily {
    Any,
    Ipv4,
    Ipv6,
}

impl AddressFamily {
    fn matches(self, address: IpAddr) -> bool {
        match self {
            Self::Any => true,
            Self::Ipv4 => address.is_ipv4(),
            Self::Ipv6 => address.is_ipv6(),
        }
    }
}

fn contains_dns_component(address: &Multiaddr) -> bool {
    address.iter().any(|protocol| {
        matches!(
            protocol,
            Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_)
        )
    })
}

fn next_dns_component(address: &Multiaddr) -> io::Result<Option<(usize, String, AddressFamily)>> {
    for (index, protocol) in address.iter().enumerate() {
        let component = match protocol {
            Protocol::Dns(name) => Some((name.into_owned(), AddressFamily::Any)),
            Protocol::Dns4(name) => Some((name.into_owned(), AddressFamily::Ipv4)),
            Protocol::Dns6(name) => Some((name.into_owned(), AddressFamily::Ipv6)),
            Protocol::Dnsaddr(name) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("/dnsaddr/{name} is not supported by the iOS system resolver"),
                ));
            }
            _ => None,
        };
        if let Some((name, family)) = component {
            return Ok(Some((index, name, family)));
        }
    }
    Ok(None)
}

async fn resolve_multiaddr(address: Multiaddr) -> io::Result<Vec<Multiaddr>> {
    let mut unresolved = vec![address];
    let mut resolved = Vec::new();
    let mut lookups = 0;

    while let Some(address) = unresolved.pop() {
        let Some((index, name, family)) = next_dns_component(&address)? else {
            resolved.push(address);
            if resolved.len() == MAX_RESOLVED_ADDRESSES {
                break;
            }
            continue;
        };

        if lookups == MAX_DNS_LOOKUPS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "multiaddr requires too many DNS lookups",
            ));
        }
        lookups += 1;

        let ips = system_lookup(&name, family).await.map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("system DNS lookup failed for {name}: {error}"),
            )
        })?;
        let replacements = replace_dns_component(&address, index, family, ips);
        if replacements.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("system DNS returned no matching addresses for {name}"),
            ));
        }

        for replacement in replacements.into_iter().rev() {
            if unresolved.len() + resolved.len() == MAX_RESOLVED_ADDRESSES {
                break;
            }
            unresolved.push(replacement);
        }
    }

    Ok(resolved)
}

async fn system_lookup(name: &str, family: AddressFamily) -> io::Result<Vec<IpAddr>> {
    let addresses = tokio::net::lookup_host((name, 0)).await?;
    let mut seen = HashSet::new();
    Ok(addresses
        .map(|address| address.ip())
        .filter(|address| family.matches(*address))
        .filter(|address| seen.insert(*address))
        .take(MAX_RESOLVED_ADDRESSES)
        .collect())
}

fn replace_dns_component(
    address: &Multiaddr,
    index: usize,
    family: AddressFamily,
    ips: impl IntoIterator<Item = IpAddr>,
) -> Vec<Multiaddr> {
    ips.into_iter()
        .filter(|ip| family.matches(*ip))
        .take(MAX_RESOLVED_ADDRESSES)
        .filter_map(|ip| address.replace(index, |_| Some(Protocol::from(ip))))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn dns4_keeps_only_ipv4_results() {
        let address: Multiaddr = "/dns4/relay.example.com/tcp/443".parse().unwrap();
        let replaced = replace_dns_component(
            &address,
            0,
            AddressFamily::Ipv4,
            [
                IpAddr::V6(Ipv6Addr::LOCALHOST),
                IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            ],
        );

        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].to_string(), "/ip4/192.0.2.1/tcp/443");
    }

    #[test]
    fn family_filtering_precedes_the_result_cap() {
        let address: Multiaddr = "/dns4/relay.example.com/tcp/443".parse().unwrap();
        let mut ips = vec![IpAddr::V6(Ipv6Addr::LOCALHOST); MAX_RESOLVED_ADDRESSES];
        ips.push(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)));

        let replaced = replace_dns_component(&address, 0, AddressFamily::Ipv4, ips);

        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].to_string(), "/ip4/192.0.2.1/tcp/443");
    }

    #[test]
    fn dns_keeps_ipv4_and_ipv6_results() {
        let address: Multiaddr = "/dns/relay.example.com/tcp/443".parse().unwrap();
        let replaced = replace_dns_component(
            &address,
            0,
            AddressFamily::Any,
            [
                IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ],
        );

        assert_eq!(replaced.len(), 2);
        assert_eq!(replaced[0].to_string(), "/ip4/192.0.2.1/tcp/443");
        assert_eq!(replaced[1].to_string(), "/ip6/::1/tcp/443");
    }

    #[test]
    fn replacement_preserves_the_complete_relay_route() {
        let relay = libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id();
        let target = libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id();
        let address: Multiaddr =
            format!("/dns4/relay.example.com/tcp/443/p2p/{relay}/p2p-circuit/p2p/{target}")
                .parse()
                .unwrap();
        let replaced = replace_dns_component(
            &address,
            0,
            AddressFamily::Ipv4,
            [IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))],
        );

        assert_eq!(
            replaced[0].to_string(),
            format!("/ip4/192.0.2.1/tcp/443/p2p/{relay}/p2p-circuit/p2p/{target}")
        );
    }

    #[test]
    fn dns6_keeps_only_ipv6_results() {
        let address: Multiaddr = "/dns6/relay.example.com/tcp/443".parse().unwrap();
        let replaced = replace_dns_component(
            &address,
            0,
            AddressFamily::Ipv6,
            [
                IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ],
        );

        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].to_string(), "/ip6/::1/tcp/443");
    }

    #[test]
    fn dnsaddr_fails_explicitly() {
        let address: Multiaddr = "/dnsaddr/bootstrap.example.com".parse().unwrap();
        let error = next_dns_component(&address).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("/dnsaddr/"));
    }
}
