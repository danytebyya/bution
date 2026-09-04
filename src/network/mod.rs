//! Local interface inventory and safe LAN route candidates.

mod benchmark;

pub use benchmark::{
    BandwidthStats, BenchmarkServer, LatencyStats, NetworkBenchmark, Stability, measure_bandwidth,
    measure_latency, run_network_benchmark,
};

use anyhow::{Context, Result};
use if_addrs::{IfAddr, get_if_addrs};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceKind {
    Ethernet,
    Wifi,
    Thunderbolt,
    UsbEthernet,
    Vpn,
    Other,
}

impl std::fmt::Display for InterfaceKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Ethernet => "Ethernet",
            Self::Wifi => "Wi-Fi",
            Self::Thunderbolt => "Thunderbolt Bridge",
            Self::UsbEthernet => "USB Ethernet",
            Self::Vpn => "VPN",
            Self::Other => "LAN",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub kind: InterfaceKind,
    pub address: IpAddr,
    pub prefix_len: u8,
    pub is_vpn: bool,
}

impl NetworkInterface {
    pub fn usable_for_cluster(&self) -> bool {
        !self.is_vpn && is_lan_address(self.address)
    }

    pub fn reaches(&self, remote: IpAddr) -> bool {
        same_subnet(self.address, remote, self.prefix_len)
    }
}

pub fn interfaces() -> Result<Vec<NetworkInterface>> {
    let mut result = Vec::new();
    for interface in get_if_addrs().context("could not enumerate network interfaces")? {
        if interface.is_loopback() {
            continue;
        }
        let (address, prefix_len) = match interface.addr {
            IfAddr::V4(address) => (IpAddr::V4(address.ip), address.prefixlen),
            IfAddr::V6(address) => (IpAddr::V6(address.ip), address.prefixlen),
        };
        let kind = classify_interface(&interface.name);
        result.push(NetworkInterface {
            name: interface.name,
            kind,
            address,
            prefix_len,
            is_vpn: kind == InterfaceKind::Vpn,
        });
    }
    result.sort_by_key(|interface| (interface.is_vpn, interface.name.clone(), interface.address));
    Ok(result)
}

pub fn route_candidates(
    local: &[NetworkInterface],
    remote_addresses: &[IpAddr],
) -> Vec<(NetworkInterface, IpAddr)> {
    let mut routes = Vec::new();
    for interface in local
        .iter()
        .filter(|interface| interface.usable_for_cluster())
    {
        for &remote in remote_addresses {
            if interface.reaches(remote) {
                routes.push((interface.clone(), remote));
            }
        }
    }
    routes
}

pub fn classify_interface(name: &str) -> InterfaceKind {
    let name = name.to_ascii_lowercase();
    if ["utun", "tun", "tap", "wg", "ppp", "tailscale", "zerotier"]
        .iter()
        .any(|prefix| name.starts_with(prefix) || name.contains(prefix))
    {
        InterfaceKind::Vpn
    } else if name.contains("thunderbolt") || name.starts_with("bridge") {
        InterfaceKind::Thunderbolt
    } else if name.contains("usb") {
        InterfaceKind::UsbEthernet
    } else if name.contains("wi-fi")
        || name.contains("wifi")
        || name.contains("wlan")
        || name.starts_with("wl")
        || name == "en0"
    {
        InterfaceKind::Wifi
    } else if name.contains("ethernet") || name.starts_with("eth") || name.starts_with("en") {
        InterfaceKind::Ethernet
    } else {
        InterfaceKind::Other
    }
}

pub fn is_lan_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_private() || address.is_link_local(),
        IpAddr::V6(address) => address.is_unique_local() || address.is_unicast_link_local(),
    }
}

fn same_subnet(local: IpAddr, remote: IpAddr, prefix_len: u8) -> bool {
    match (local, remote) {
        (IpAddr::V4(local), IpAddr::V4(remote)) => {
            let prefix = prefix_len.min(32);
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            u32::from(local) & mask == u32::from(remote) & mask
        }
        (IpAddr::V6(local), IpAddr::V6(remote)) => {
            let prefix = prefix_len.min(128);
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            u128::from(local) & mask == u128::from(remote) & mask
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn recognizes_private_and_link_local_lan_addresses() {
        assert!(is_lan_address(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 4))));
        assert!(is_lan_address(IpAddr::V4(Ipv4Addr::new(169, 254, 1, 4))));
        assert!(is_lan_address(IpAddr::V6(
            "fe80::1".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(!is_lan_address(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn excludes_common_vpn_interfaces() {
        assert_eq!(classify_interface("utun4"), InterfaceKind::Vpn);
        assert_eq!(classify_interface("Tailscale Tunnel"), InterfaceKind::Vpn);
        assert_eq!(classify_interface("Ethernet 2"), InterfaceKind::Ethernet);
    }

    #[test]
    fn checks_subnet_before_using_route() {
        assert!(same_subnet(
            "192.168.1.4".parse().unwrap(),
            "192.168.1.18".parse().unwrap(),
            24,
        ));
        assert!(!same_subnet(
            "192.168.1.4".parse().unwrap(),
            "192.168.2.18".parse().unwrap(),
            24,
        ));
    }
}
