#![warn(unused_extern_crates)]
extern crate tun_tap;
#[macro_use] extern crate log;
extern crate env_logger;
extern crate pktparse;
extern crate nom;
extern crate pnet;
#[macro_use] extern crate structopt;
#[macro_use] extern crate error_chain;

mod errors {
    error_chain! {
        foreign_links {
            Io(::std::io::Error);
        }
    }
}
// use errors::{Result, Error, ErrorKind};
use errors::{Result, ResultExt};

use tun_tap::Iface;
use tun_tap::Mode::Tap;

use nom::IResult::Done;
use pktparse::arp;
use pktparse::ipv4;
use pktparse::ethernet::{self, EtherType};
use structopt::StructOpt;

use std::thread;
use std::sync::Arc;
use std::process::Command;
use pnet::datalink::{self, NetworkInterface, MacAddr, DataLinkSender, DataLinkReceiver};
use pnet::datalink::Channel::Ethernet;

#[derive(StructOpt, Debug)]
struct Opt {
    tun: String,
    #[structopt(default_value="burritun0")]
    tap: String,
}

/// Run a shell command. Panic if it fails in any way.
fn cmd(cmd: &str, args: &[&str]) {
    let ecode = Command::new("ip")
        .args(args)
        .spawn()
        .unwrap()
        .wait()
        .unwrap();
    assert!(ecode.success(), "Failed to execte {}", cmd);
}

fn open_tun(tun: &str) -> Result<(Box<DataLinkSender>, Box<DataLinkReceiver>)> {
    let interfaces = datalink::interfaces();
    let interface = interfaces.into_iter()
        .filter(|iface: &NetworkInterface| iface.name == tun)
        .next()
        .chain_err(|| "Interface not found")?;

    let (tx, rx) = match datalink::channel(&interface, Default::default()) {
        Ok(Ethernet(tx, rx)) => (tx, rx),
        Ok(_) => bail!("Unhandled channel type"),
        Err(e) => bail!("An error occurred when creating the datalink channel: {}", e)
    };

    info!("opened tun device: {:?}", tun);

    Ok((tx, rx))
}

fn open_tap(tap: &str) -> Result<(Arc<Iface>, Arc<Iface>)> {
    let iface = Iface::new(&tap, Tap)?;
    info!("opened tap device: {:?}", iface.name());
    cmd("ip", &["link", "set", "up", "dev", iface.name()]);

    let tap = Arc::new(iface);
    let tap_writer = Arc::clone(&tap);
    let tap_reader = Arc::clone(&tap);

    Ok((tap_writer, tap_reader))
}

fn iface_mac(name: &str) -> Result<MacAddr> {
    let interfaces = datalink::interfaces();
    let interface = interfaces.into_iter()
        .filter(|iface: &NetworkInterface| iface.name == name)
        .next()
        .chain_err(|| "Interface not found")?;
    Ok(interface.mac.unwrap())
}

fn tun2tap(mac: MacAddr, mut tun_rx: Box<DataLinkReceiver>, tap_tx: Arc<Iface>) -> Result<()> {
    while let Ok(packet) = tun_rx.next() {
        debug!("recv(tun): {:?}", packet);
        if let Done(_remaining, ipv4_hdr) = ipv4::parse_ipv4_header(&packet) {
            debug!("recv(tun, ipv4): {:?}", ipv4_hdr);

            let mut out = vec![
                0, 0, 0, 0,                                 // ????
                mac.0, mac.1, mac.2, mac.3, mac.4, mac.5,   // dest mac
                0x11, 0x22, 0x33, 0x44, 0x55, 0x66,         // src mac
                8, 0,                                       // ipv4
            ];
            out.extend(packet);

            debug!("send(tap): {:?}", out);
            tap_tx.send(&out)?;
        }
    }

    Ok(())
}

fn tap2tun(tap: Arc<Iface>, mut tun_tx: Box<DataLinkSender>) -> Result<()> {
    let mut buffer = vec![0; 1504]; // MTU + 4 for the header
    loop {
        let n = tap.recv(&mut buffer)?;
        debug!("recv(tap): {:?}", &buffer[4..n]);

        if let Done(remaining, eth_frame) = ethernet::parse_ethernet_frame(&buffer[4..n]) {
            debug!("recv(tap, eth): {:?}, {:?}", eth_frame, remaining);

            match eth_frame.ethertype {
                EtherType::ARP => {
                    if let Done(_, arp_pkt) = arp::parse_arp_pkt(remaining) {
                        info!("recv(tap, arp): {:?}", arp_pkt);

                        let mut out = vec![
                            0, 0, 0, 0,                     // ????
                            255, 255, 255, 255, 255, 255,   // dest mac
                            255, 255, 255, 255, 255, 255,   // src mac
                            8, 6,                           // arp

                            0, 1,                           // hw addr: eth
                            8, 0,                           // proto addr: ipv4
                            6, 4,                           // sizes
                            0, 2,                           // operation: reply
                        ];

                        out.extend(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]); // src mac
                        out.extend(&arp_pkt.dest_addr.octets()); // src ip

                        out.extend(&arp_pkt.src_mac.0); // dest mac
                        out.extend(&arp_pkt.src_addr.octets()); // dest ip

                        info!("send(tap, arp): {:?}", &out);
                        tap.send(&out)?;
                    }
                },

                EtherType::IPv4 => {
                    if let Done(payload, ip_hdr) = ipv4::parse_ipv4_header(remaining) {
                        debug!("send(tun, ipv4): {:?}, {:?}", ip_hdr, payload);
                    }

                    debug!("send(tun): {:?}", remaining);
                    tun_tx.send_to(&remaining, None).unwrap()?;
                },

                _ => (),
            }
        }
    }
}

fn run() -> Result<()> {
    let env = env_logger::Env::default()
        .filter_or("RUST_LOG", "debug")
        .write_style("MY_LOG_STYLE");
    env_logger::init_from_env(env);

    let opt = Opt::from_args();
    debug!("{:?}", opt);

    let (tap_tx, tap_rx) = open_tap(&opt.tap)?;
    let (tun_tx, tun_rx) = open_tun(&opt.tun)?;

    let mac = iface_mac(&opt.tap)?;
    info!("using {:?} as local mac address", mac);

    let t1 = thread::spawn(move || {
        tun2tap(mac, tun_rx, tap_tx)
    });

    let t2 = thread::spawn(move || {
        tap2tun(tap_rx, tun_tx)
    });

    t1.join()
        .map_err(|_| "tun2tap thread failed")??;
    t2.join()
        .map_err(|_| "tap2tun thread failed")??;

    Ok(())
}

fn main() {
    if let Err(ref e) = run() {
        use error_chain::ChainedError; // trait which holds `display_chain`
        eprintln!("{}", e.display_chain());
        ::std::process::exit(1);
    }
}
