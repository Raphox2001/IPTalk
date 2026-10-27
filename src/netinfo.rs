//! Eigene IPv4-Adressen im lokalen Netzwerk ermitteln (für die Anzeige im Fenster).

use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAddr {
    pub ip: Ipv4Addr,
    /// Name des Netzwerkadapters, z. B. „Ethernet“ oder „enp5s0“.
    pub interface: String,
}

/// Adapter, die fast nie das gemeinsame Heimnetz sind (VPN, Container, virtuelle Maschinen).
const VIRTUAL_HINTS: &[&str] = &[
    "vethernet", "virtual", "vpn", "tap", "tun", "wsl", "hyper-v", "docker", "vmware", "virtualbox",
    "vbox", "tailscale", "zerotier", "wireguard", "wg", "br-", "virbr", "veth", "openvpn", "surfshark",
    "nordlynx", "proton", "bluetooth",
];

fn is_virtual(name: &str) -> bool {
    let n = name.to_lowercase();
    VIRTUAL_HINTS.iter().any(|h| {
        if h.len() > 3 {
            n.contains(h)
        } else {
            // Kurze Kürzel („tun“, „wg“) nur am Wortanfang, sonst träfe z. B. „tun“ auch andere Namen.
            n.starts_with(h) || n.contains(&format!(" {h}")) || n.contains(&format!("({h}"))
        }
    })
}

/// Sortierschlüssel: kleiner = wahrscheinlicher die richtige Adresse fürs Heimnetz.
fn rank(a: &LocalAddr, p2p: bool) -> u8 {
    let o = a.ip.octets();
    let range = match o {
        [192, 168, ..] => 0,
        [10, ..] => 1,
        [172, b, ..] if (16..=31).contains(&b) => 2,
        _ => 3,
    };
    let virt = if is_virtual(&a.interface) || p2p { 4 } else { 0 };
    range + virt
}

fn usable(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() && !ip.is_multicast()
}

/// Alle nutzbaren IPv4-Adressen, die wahrscheinlichste zuerst.
pub fn local_ipv4() -> Vec<LocalAddr> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else { return Vec::new() };
    let mut list: Vec<(u8, LocalAddr)> = ifaces
        .into_iter()
        .filter(|i| i.is_oper_up())
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(ip) if usable(ip) => {
                let a = LocalAddr { ip, interface: i.name.clone() };
                Some((rank(&a, i.is_p2p()), a))
            }
            _ => None,
        })
        .collect();
    list.sort_by_key(|(r, a)| (*r, a.ip));
    list.dedup_by(|x, y| x.1.ip == y.1.ip);
    list.into_iter().map(|(_, a)| a).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(ip: [u8; 4], name: &str) -> LocalAddr {
        LocalAddr { ip: Ipv4Addr::from(ip), interface: name.into() }
    }

    #[test]
    fn home_lan_beats_vpn_and_wsl() {
        let lan = rank(&addr([192, 168, 0, 168], "Ethernet"), false);
        assert!(lan < rank(&addr([10, 8, 0, 2], "Surfshark WireGuard"), false));
        assert!(lan < rank(&addr([172, 20, 48, 1], "vEthernet (WSL)"), false));
        assert!(lan < rank(&addr([10, 5, 0, 2], "tun0"), true));
        assert!(rank(&addr([10, 0, 0, 5], "enp5s0"), false) < rank(&addr([192, 168, 122, 1], "virbr0"), false));
    }

    #[test]
    fn filters_unusable() {
        assert!(!usable(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(!usable(Ipv4Addr::new(169, 254, 205, 184)));
        assert!(usable(Ipv4Addr::new(192, 168, 0, 168)));
    }

    #[test]
    fn plain_adapter_names_are_not_virtual() {
        for n in ["Ethernet", "Ethernet 2", "WLAN", "enp5s0", "eth0", "wlp3s0", "eno1"] {
            assert!(!is_virtual(n), "{n}");
        }
    }
}
