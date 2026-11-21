//! The oracle build reproduces the strict aoTuV streams measured before the port (q 0.5; the
//! SHA-256 of the whole Ogg stream): thetest16/44/48 mono and the
//! 44.1 kHz stereo input; and the Lancer copy its 44.1 kHz mono stream (`920a75b0…`, the cc
//! release build: proves the baseline is Lancer as `vorbis_rs` builds it). One summary line.
use aotuv_sys::{Mode, STRICT, encode, lancer, spike_pcm, thetest};
use std::io::Write;
use std::process::{Command, Stdio};

const HASHES: [(u32, usize, &str); 4] = [
    (
        16,
        1,
        "ed0a63e6dfeca084281674b5d2e32bbb6be01c42f085e06f7c7c30059a62458d",
    ),
    (
        44,
        1,
        "bb18461c79be3acfb6bdaea18b8342dc3fd50a9f4b88372540b565878d610528",
    ),
    (
        48,
        1,
        "b14adf6a90bb257b9a8bffb618d694e85f8fdbbcd071f957a890b2e43ca13afc",
    ),
    (
        44,
        2,
        "e9485694522b85ad0666a13de175ef44ac4e00dd0ed9005b50fefeca65cff547",
    ),
];

fn sha256(d: &[u8]) -> String {
    let mut c = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("sha256sum");
    c.stdin.take().unwrap().write_all(d).unwrap();
    let o = c.wait_with_output().unwrap();
    String::from_utf8_lossy(&o.stdout)[..64].to_string()
}

#[test]
fn spike_hashes() {
    for (khz, ch, want) in HASHES {
        let (rate, pcm) = thetest(khz);
        let s = encode(STRICT, rate as i64, Mode::Vbr(0.5), &spike_pcm(&pcm, ch)).unwrap();
        assert_eq!(sha256(&s), want, "thetest{khz} × {ch} ch");
    }
    let l = match lancer::API {
        Some(api) => {
            let (rate, pcm) = thetest(44);
            let s = encode(api, rate as i64, Mode::Vbr(0.5), &spike_pcm(&pcm, 1)).unwrap();
            assert_eq!(&sha256(&s)[..8], "920a75b0", "Lancer thetest44");
            "; Lancer's 44.1 kHz mono too"
        }
        None => "; no Lancer copy on this target",
    };
    println!(
        "aotuv-sys: the spike's 4 strict streams reproduce (thetest16/44/48 mono, 44.1 kHz stereo){l}"
    );
}
