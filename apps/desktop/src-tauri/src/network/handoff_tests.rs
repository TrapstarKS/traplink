use super::*;
use crate::flow::SwitchEvent;

// All native movement on these receivers goes to input/test_display.rs.
// The two cores use real pairing, encrypted TCP, input leases and returns.
#[tokio::test]
async fn rapid_handoffs_over_an_authenticated_link() {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let a = Core::for_test(a_dir.path().to_owned(), address.port());
    let b = Core::for_test(b_dir.path().to_owned(), 48372);
    a.listening.store(true, Ordering::Relaxed);
    let invite = create_invite_for_addresses(&a, vec![address.to_string()]).unwrap();
    let (accepted, connected) = tokio::join!(
        async {
            let (stream, address) = listener.accept().await.unwrap();
            accept(a.clone(), stream, address).await
        },
        connect_invitation(b.clone(), &invite.invitation)
    );
    accepted.unwrap();
    let a_id = connected.unwrap().id;
    let b_id = b.data.lock().unwrap().device_id.clone();
    assert!(
        flow_start(&b, &a_id, Uuid::new_v4(), FlowEdge::Right, 0.5, false)
            .await
            .is_err()
    );
    for core in [&a, &b] {
        let mut data = core.data.lock().unwrap();
        data.settings.input_enabled = true;
        data.peers[0].peer.permissions.input = true;
        data.peers[0].peer.permissions.files = true;
    }
    let path = a_dir.path().join("concurrent-transfer.bin");
    let payload: Vec<u8> = (0..CHUNK * 64).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &payload).unwrap();
    let file_transfer = tokio::spawn({
        let (sender, target) = (a.clone(), b_id.clone());
        async move { send_file(sender, &target, &path).await }
    });
    let edges = [
        FlowEdge::Right,
        FlowEdge::Left,
        FlowEdge::Top,
        FlowEdge::Bottom,
    ];
    let mut previous = Uuid::new_v4();
    let mut start_ack_ms = Vec::new();
    let mut return_ack_ms = Vec::new();
    for visit in 0..96 {
        let (sender, target, receiver, origin) = if visit % 2 == 0 {
            (&a, &b_id, &b, &a_id)
        } else {
            (&b, &a_id, &a, &b_id)
        };
        let edge = edges[(visit / 2) % edges.len()];
        let entry = [0.0, 0.5, 1.0][visit % 3];
        let link = session(sender, target).unwrap();
        let token = {
            let mut input = sender.input.lock().unwrap();
            assert!(!input.busy());
            input.sending_to = Some(target.clone());
            input
                .flow
                .begin_switch(target, edge, Instant::now(), link)
                .unwrap()
        };
        let triggered = Instant::now();
        flow_start(sender, target, token, edge, entry, false)
            .await
            .unwrap();
        start_ack_ms.push(triggered.elapsed().as_millis() as u64);
        assert!(sender
            .input
            .lock()
            .unwrap()
            .flow
            .switch
            .apply(SwitchEvent::Started {
                peer: target.clone(),
                ok: true,
            }));

        // Simulate callbacks left over from a previous visit in both directions.
        send_flow(sender, target, previous, FlowPacket::Stop)
            .await
            .unwrap();
        send_flow(sender, target, previous, FlowPacket::Keepalive)
            .await
            .unwrap();
        send_flow(receiver, origin, previous, FlowPacket::Return { entry })
            .await
            .unwrap();
        send_flow(receiver, origin, previous, FlowPacket::Stop)
            .await
            .unwrap();
        if visit % 4 == 0 {
            // An untagged stop from a closed manual-control pane must also
            // leave a newer Flow visit alone on protocol 4.
            try_send(receiver, origin, Wire::ControlStop);
            try_send(sender, target, Wire::ControlStop);
        }
        // An acknowledged request provides a FIFO barrier on each stream.
        for (from, to) in [(sender, target), (receiver, origin)] {
            request(
                &session(from, to).unwrap(),
                Wire::InputConsent {
                    request: Uuid::new_v4().to_string(),
                    flow: true,
                },
            )
            .await
            .unwrap();
        }
        assert_eq!(sender.input.lock().unwrap().flow.switch_id(), Some(token));
        assert!(crate::input::active_with(receiver, origin));
        match visit % 12 {
            0 => release_flow_control(receiver, origin, token).await.unwrap(),
            1 => release_flow_control(sender, target, token).await.unwrap(),
            _ => {
                assert!(release_flow_control(sender, target, previous)
                    .await
                    .is_err());
                let (dx, dy) = match edge {
                    FlowEdge::Right => (32, 0),
                    FlowEdge::Left => (-32, 0),
                    FlowEdge::Top => (0, -32),
                    FlowEdge::Bottom => (0, 32),
                };
                for (dx, dy) in [(dx, dy), (-4 * dx, -4 * dy)] {
                    send_flow(
                        sender,
                        target,
                        token,
                        FlowPacket::Input {
                            event: InputEvent::MoveBy { dx, dy },
                        },
                    )
                    .await
                    .unwrap();
                }
            }
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while crate::input::active_with(sender, target)
                || crate::input::active_with(receiver, origin)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("both computers must release this visit without Repair connection");
        if visit % 12 > 1 {
            let input = receiver.input.lock().unwrap();
            let measurement = input.last_return.as_ref().expect("confirmed return timing");
            assert_eq!(&measurement.peer, origin);
            return_ack_ms.push(measurement.elapsed_ms);
        }
        previous = token;
    }
    assert_eq!(file_transfer.await.unwrap().unwrap().status, "complete");
    let received = std::fs::read_dir(b_dir.path().join("downloads").join("TrapLink"))
        .unwrap()
        .map(|entry| entry.unwrap())
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with("concurrent-transfer.bin")
        })
        .expect("the file must reach the other computer during rapid handoffs");
    assert!(std::fs::read(received.path()).unwrap() == payload);
    for (phase, samples) in [
        ("start acknowledgement", &mut start_ack_ms),
        ("return confirmation", &mut return_ack_ms),
    ] {
        samples.sort_unstable();
        eprintln!(
            "Loopback {phase}: {} samples, median {} ms, p95 {} ms, maximum {} ms",
            samples.len(),
            samples[samples.len() / 2],
            samples[(samples.len() * 95).div_ceil(100) - 1],
            samples.last().unwrap()
        );
    }
    disconnect(&a, &b_id).unwrap();
}
