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
use pktparse::ethernet;
use pktparse::ipv4;
use structopt::StructOpt;

use std::thread;
use std::process::Command;
// use std::sync::mpsc;
use std::sync::Arc;
// use pnet::transport::TransportChannelType::Layer3;
use pnet::datalink::{self, NetworkInterface, DataLinkSender, DataLinkReceiver};
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

fn tun2tap(mut tun_rx: Box<DataLinkReceiver>, tap_tx: Arc<Iface>) -> Result<()> {
    while let Ok(packet) = tun_rx.next() {
        debug!("recv(tun): {:?}", packet);
        if let Done(_remaining, ipv4_hdr) = ipv4::parse_ipv4_header(&packet) {
            debug!("recv(tun, ipv4): {:?}", ipv4_hdr);

            let mut out = vec![
                0, 0, 0, 0,                     // ????
                255, 255, 255, 255, 255, 255,   // dest mac
                255, 255, 255, 255, 255, 255,   // src mac
                8, 0,                           // ipv4
            ];
            out.extend(packet);

            debug!("send(tap): {:?}", out);
            tap_tx.send(&out)?;
        }
    }

    Ok(())
}

fn tap2tun(tap_rx: Arc<Iface>, _tun_tx: Box<DataLinkSender>) -> Result<()> {
    let mut buffer = vec![0; 1504]; // MTU + 4 for the header
    loop {
        let n = tap_rx.recv(&mut buffer)?;
        debug!("recv(tap): {:?}", &buffer[4..n]);

        if let Done(remaining, eth_frame) = ethernet::parse_ethernet_frame(&buffer[4..n]) {
            debug!("recv(tap, eth): {:?}, {:?}", eth_frame, remaining);
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

    let t1 = thread::spawn(move || {
        tun2tap(tun_rx, tap_tx)
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
