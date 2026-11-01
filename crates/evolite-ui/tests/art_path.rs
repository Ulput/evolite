//! The whole way of an NFT picture, as the app takes it: the core is asked to draw it, the
//! sandboxed renderer (the app's own executable) draws its first frame and then its animation,
//! and the second time the picture comes from the disk without the renderer. Through the demo
//! core: with `--features dev-tools`, or without `real`.

#![cfg(demo_core)]

use std::path::Path;
use std::time::{Duration, Instant};

use evolite_api::{Command, Event, Nft, Picture};
use evolite_core::{ArtOptions, Core};
use evolite_render::{Settings, worker};
use std::sync::Arc;

fn core(cache: &Path) -> Core {
    Core::mock(
        ArtOptions {
            cache: Some(cache.to_path_buf()),
            worker: Some(Settings::new(
                env!("CARGO_BIN_EXE_evolite").into(),
                vec![worker::FLAG.into()],
            )),
        },
        || {},
    )
}

/// The demo NFTs, from the core's first snapshot.
fn demo_nfts(core: &Core) -> Vec<Nft> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for event in core.events() {
            if let Event::Snapshot(s) = event {
                return s.wallet.expect("the demo wallet").nfts;
            }
        }
        assert!(Instant::now() < deadline, "no snapshot");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Every `Art` answer for `art` until a whole one comes: (partial, picture) in order. Answers
/// for other pictures stay in `backlog` for their turn.
fn answers(core: &Core, backlog: &mut Vec<Event>, art: &str) -> Vec<(bool, Arc<Picture>)> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut got = Vec::new();
    loop {
        backlog.extend(core.events());
        let (mine, rest): (Vec<Event>, Vec<Event>) = std::mem::take(backlog)
            .into_iter()
            .partition(|e| matches!(e, Event::Art { art: a, .. } if a == art));
        *backlog = rest;
        for event in mine {
            if let Event::Art {
                art: a,
                partial,
                result,
                ..
            } = event
                && a == art
            {
                got.push((partial, result.expect("drawn")));
                if !partial {
                    return got;
                }
            }
        }
        assert!(Instant::now() < deadline, "no whole picture for {art}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn nft<'a>(nfts: &'a [Nft], id: &str) -> &'a Nft {
    nfts.iter().find(|n| n.id == id).expect("a demo NFT")
}

#[test]
fn a_picture_shows_its_first_frame_then_its_animation_then_comes_from_the_disk() {
    let cache = std::env::temp_dir().join(format!("evolite-art-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    let first = core(&cache);
    let nfts = demo_nfts(&first);
    let animated = nft(&nfts, "nft-coffin-dance-7");
    let still = nft(&nfts, "nft-teddy-19");
    for nft in [animated, still] {
        first.send(Command::DrawArt {
            art: nft.art.clone(),
            side: 96,
            urgent: true,
        });
    }

    let mut backlog = Vec::new();
    let drawn = answers(&first, &mut backlog, &animated.art);
    assert_eq!(drawn.len(), 2, "the first frame, then the animation");
    let (partial, start) = &drawn[0];
    assert!(*partial && start.frames.len() == 1);
    let (_, whole) = &drawn[1];
    assert!(whole.frames.len() > 1, "an animation");
    assert_eq!(whole.width.max(whole.height), 96, "drawn at the size asked");
    // Every frame unpacks to its size.
    let mut rgba = Vec::new();
    assert!((0..whole.frames.len()).all(|i| whole.unpack(i, &mut rgba)));

    // A still is whole after its first pass: nothing more comes.
    let still_answers = answers(&first, &mut backlog, &still.art);
    assert_eq!(still_answers.len(), 1);
    assert_eq!(still_answers[0].1.frames[0].delay_ms, 0);
    drop(first);

    // A new run: from the disk, whole at once, the same frames.
    let second = core(&cache);
    let _ = demo_nfts(&second);
    second.send(Command::DrawArt {
        art: animated.art.clone(),
        side: 96,
        urgent: true,
    });
    let again = answers(&second, &mut Vec::new(), &animated.art);
    assert_eq!(again.len(), 1, "no first frame: it is on the disk");
    assert_eq!(*again[0].1, **whole);
    let _ = std::fs::remove_dir_all(&cache);
}

#[test]
fn a_picture_the_wallet_does_not_have_is_refused() {
    let cache = std::env::temp_dir().join(format!("evolite-art-none-{}", std::process::id()));
    let core = core(&cache);
    let _ = demo_nfts(&core);
    core.send(Command::DrawArt {
        art: "00".repeat(32),
        side: 96,
        urgent: true,
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if core
            .events()
            .iter()
            .any(|e| matches!(e, Event::Art { result: Err(_), .. }))
        {
            break;
        }
        assert!(Instant::now() < deadline, "no answer");
        std::thread::sleep(Duration::from_millis(20));
    }
}
