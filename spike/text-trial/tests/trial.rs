//! What each candidate does, as tests. A test that records a limit of a library asserts the behaviour that was
//! seen, so a new version that behaves differently fails here and the results file is looked at again.

use text_trial::{Am, Candidate, Loro, Rng, TEXT, Yrs, keystroke, typed};

// -----------------------------------------------------------------------------------------------------------------
// The same for every candidate
// -----------------------------------------------------------------------------------------------------------------

/// Three clients type at once, each at a random place in its own copy. Updates reach the others late, in an
/// order that differs per receiver (each sender's own updates stay in order), and some arrive twice.
fn three_clients_converge<C: Candidate>() {
    for seed in 0..5 {
        let mut rng = Rng(seed);
        let mut docs: Vec<C::Doc> = (1..=3).map(C::new).collect();
        // For each receiver, the updates it has not yet been given, one queue per sender.
        let mut queues: Vec<Vec<Vec<Vec<u8>>>> = vec![vec![Vec::new(); 3]; 3];
        let mut typed_chars = 0;
        for round in 0..120 {
            let who = rng.below(3);
            let len = C::len(&mut docs[who]);
            let update = if len > 10 && rng.below(6) == 0 {
                typed_chars -= 1;
                let at = rng.below(len);
                C::delete(&mut docs[who], at, 1)
            } else {
                typed_chars += 1;
                let at = rng.below(len + 1);
                C::insert(&mut docs[who], at, keystroke(round))
            };
            for (to, queue) in queues.iter_mut().enumerate() {
                if to != who {
                    queue[who].push(update.clone());
                }
            }
            // Now and then a receiver is given the oldest waiting update of one sender, sometimes twice.
            for _ in 0..rng.below(3) {
                let (to, from) = (rng.below(3), rng.below(3));
                if !queues[to][from].is_empty() {
                    let u = queues[to][from].remove(0);
                    C::apply(&mut docs[to], &u).unwrap();
                    if rng.below(4) == 0 {
                        C::apply(&mut docs[to], &u).unwrap();
                    }
                }
            }
        }
        // Everything still waiting is delivered, the senders taken in a different order by each receiver.
        for to in 0..3 {
            for k in 0..3 {
                let from = (to + k + seed as usize) % 3;
                for u in std::mem::take(&mut queues[to][from]) {
                    C::apply(&mut docs[to], &u).unwrap();
                }
            }
        }
        let texts: Vec<String> = docs.iter_mut().map(C::text).collect();
        assert_eq!(texts[0], texts[1], "{} seed {seed}", C::NAME);
        assert_eq!(texts[1], texts[2], "{} seed {seed}", C::NAME);
        // Two clients can delete the same character, so the length is at least what the counts say.
        assert!(texts[0].chars().count() as i64 >= typed_chars);
        // A copy loaded from one client's bytes is the same text.
        let snapshot = C::snapshot(&mut docs[0]);
        let mut loaded = C::load(9, &snapshot).unwrap();
        assert_eq!(C::text(&mut loaded), texts[0]);
    }
}

#[test]
fn three_clients_typing_at_once_end_with_the_same_text() {
    three_clients_converge::<Loro>();
    three_clients_converge::<Yrs>();
    three_clients_converge::<Am>();
}

/// The server's rule from the design: an update that would make the text too long is refused, and the server's
/// document is left as it was.
fn server_check<C: Candidate>() {
    let (_, updates) = typed::<C>(1, 30);
    let mut server = C::new(100);
    for u in &updates[..20] {
        C::checked_apply(&mut server, u, 20).unwrap();
    }
    let before = (C::text(&mut server), C::snapshot(&mut server));
    assert_eq!(before.0.chars().count(), 20);
    let refused = C::checked_apply(&mut server, &updates[20], 20);
    assert!(refused.is_err(), "{}", C::NAME);
    assert_eq!(C::text(&mut server), before.0);
    assert_eq!(
        C::snapshot(&mut server),
        before.1,
        "{}: refused update left a trace",
        C::NAME
    );
    // With a higher limit the same update is accepted.
    C::checked_apply(&mut server, &updates[20], 21).unwrap();
    assert_eq!(C::len(&mut server), 21);
}

#[test]
fn the_server_can_refuse_an_update_and_keep_its_document_unchanged() {
    server_check::<Loro>();
    server_check::<Yrs>();
    server_check::<Am>();
}

/// A client sends keystrokes 22 to 30 before keystroke 21. Each library takes the early ones without complaint
/// and parks them. The server's rule has to refuse them, or its limit means nothing.
fn out_of_order<C: Candidate>() -> (usize, usize) {
    let (_, updates) = typed::<C>(1, 30);
    // What the library does on its own: everything is "ok", and nothing shows until the missing one arrives.
    let mut plain = C::new(100);
    for u in &updates[..20] {
        assert_eq!(C::apply_whole(&mut plain, u), Ok(true));
    }
    for u in &updates[21..] {
        assert_eq!(C::apply_whole(&mut plain, u), Ok(false), "{}", C::NAME);
    }
    assert_eq!(C::len(&mut plain), 20);
    // A scratch copy taken now, given the missing keystroke: does it see what was parked?
    let mut copy = C::scratch(&mut plain);
    C::apply(&mut copy, &updates[20]).unwrap();
    let seen_by_copy = C::len(&mut copy);
    C::apply(&mut plain, &updates[20]).unwrap();
    let in_document = C::len(&mut plain);

    // The server's rule, limit 21: the early ones are refused and leave nothing behind; the honest one passes.
    let mut server = C::new(100);
    for u in &updates[..20] {
        C::checked_apply(&mut server, u, 21).unwrap();
    }
    let before = C::snapshot(&mut server);
    for u in &updates[21..] {
        assert!(C::checked_apply(&mut server, u, 21).is_err(), "{}", C::NAME);
    }
    assert_eq!(
        C::snapshot(&mut server),
        before,
        "{}: a refused update left a trace",
        C::NAME
    );
    C::checked_apply(&mut server, &updates[20], 21).unwrap();
    assert_eq!(C::len(&mut server), 21);
    assert!(C::checked_apply(&mut server, &updates[21], 21).is_err());
    assert_eq!(C::len(&mut server), 21);
    (seen_by_copy, in_document)
}

#[test]
fn updates_sent_out_of_order_do_not_get_past_the_server() {
    // In every library the document ends with all 30 characters once the missing keystroke arrives.
    // Loro's `fork` drops what was parked, so a check on the copy alone would see 21 and pass. The copies of
    // yrs and Automerge keep it and see 30.
    assert_eq!(out_of_order::<Loro>(), (21, 30));
    assert_eq!(out_of_order::<Yrs>(), (30, 30));
    assert_eq!(out_of_order::<Am>(), (30, 30));
}

// -----------------------------------------------------------------------------------------------------------------
// Question 5: can part of a document be synced?
// -----------------------------------------------------------------------------------------------------------------

#[test]
fn loro_has_no_way_to_send_one_container_of_a_document() {
    use loro::{ContainerTrait, LoroDoc, VersionVector};
    // The export modes are whole-document or by version; none names a container. So the only thing to try is to
    // take the changes apart. One peer edits two texts in turn.
    let doc = LoroDoc::new();
    doc.set_peer_id(1).unwrap();
    let (a, b) = (doc.get_text("a"), doc.get_text("b"));
    for i in 0..20 {
        a.insert(i, "a").unwrap();
        doc.commit();
        b.insert(i, "b").unwrap();
        doc.commit();
    }
    let json = doc
        .export_json_updates(&VersionVector::default(), &doc.oplog_vv())
        .unwrap();
    let mut only_a = json.clone();
    for c in &mut only_a.changes {
        c.ops.retain(|op| op.container == a.id());
    }
    only_a.changes.retain(|c| !c.ops.is_empty());
    let fresh = LoroDoc::new();
    let err = fresh.import_json_updates(only_a).unwrap_err().to_string();
    assert!(err.contains("contiguous"), "{err}");
    assert_eq!(fresh.get_text("a").to_string(), "");

    // Two peers, each editing its own text, who sync with each other now and then. The changes on `a` are all
    // one peer's, but they depend on the other peer's changes, so alone they wait for ever.
    let (p1, p2) = (LoroDoc::new(), LoroDoc::new());
    p1.set_peer_id(1).unwrap();
    p2.set_peer_id(2).unwrap();
    for i in 0..20 {
        p1.get_text("a").insert(i, "a").unwrap();
        p1.commit();
        p2.get_text("b").insert(i, "b").unwrap();
        p2.commit();
        if i % 5 == 4 {
            let u1 = p1.export(loro::ExportMode::all_updates()).unwrap();
            let u2 = p2.export(loro::ExportMode::all_updates()).unwrap();
            p1.import(&u2).unwrap();
            p2.import(&u1).unwrap();
        }
    }
    let mut only_a = p1
        .export_json_updates(&VersionVector::default(), &p1.oplog_vv())
        .unwrap();
    let a_id = p1.get_text("a").id();
    for c in &mut only_a.changes {
        c.ops.retain(|op| op.container == a_id);
    }
    only_a.changes.retain(|c| !c.ops.is_empty());
    let fresh = LoroDoc::new();
    let status = fresh.import_json_updates(only_a).unwrap();
    assert!(status.pending.is_some(), "changes on `a` were all applied");
    // Only what was typed before the first sync arrives.
    assert_eq!(fresh.get_text("a").to_string(), "aaaaa");
}

#[test]
fn yrs_syncs_part_of_a_document_only_as_a_sub_document() {
    use yrs::updates::decoder::Decode;
    use yrs::{Doc, GetString, Map, ReadTxn, StateVector, Text, Transact, Update};
    let state = |d: &Doc| {
        d.transact()
            .encode_state_as_update_v1(&StateVector::default())
    };
    // A sub-document is a document of its own that the parent only points to.
    let root = Doc::new();
    let objects = root.get_or_insert_map("objects");
    let sub = objects.insert(&mut root.transact_mut(), "x", Doc::new());
    let before = state(&root).len();
    let text = sub.get_or_insert_text(TEXT);
    for i in 0..200 {
        text.insert(&mut sub.transact_mut(), i, "s");
    }
    assert_eq!(
        state(&root).len(),
        before,
        "the parent's state grew with the sub-document"
    );

    // A copy given only the parent has the pointer and none of the text.
    let copy = Doc::new();
    let copy_objects = copy.get_or_insert_map("objects");
    copy.transact_mut()
        .apply_update(Update::decode_v1(&state(&root)).unwrap())
        .unwrap();
    let Some(yrs::Out::YDoc(copy_sub)) = copy_objects.get(&copy.transact(), "x") else {
        panic!("the copy has no sub-document");
    };
    assert_eq!(copy_sub.guid(), sub.guid());
    let copy_text = copy_sub.get_or_insert_text(TEXT);
    assert_eq!(copy_text.get_string(&copy_sub.transact()).len(), 0);
    // The text arrives with the sub-document's own update, exchanged like any other document's.
    copy_sub
        .transact_mut()
        .apply_update(Update::decode_v1(&state(&sub)).unwrap())
        .unwrap();
    assert_eq!(copy_text.get_string(&copy_sub.transact()).len(), 200);
}

#[test]
fn automerge_changes_for_one_object_wait_for_all_the_others() {
    use automerge::transaction::Transactable;
    use automerge::{AutoCommit, ObjType, ROOT, ReadDoc};
    let mut doc = AutoCommit::new();
    let a = doc.put_object(ROOT, "a", ObjType::Text).unwrap();
    let b = doc.put_object(ROOT, "b", ObjType::Text).unwrap();
    doc.commit();
    for i in 0..20 {
        doc.splice_text(&a, i, 0, "a").unwrap();
        doc.commit();
        doc.splice_text(&b, i, 0, "b").unwrap();
        doc.commit();
    }
    let changes = doc.get_changes(&[]);
    assert_eq!(changes.len(), 41);
    // The change that made the two objects, and every change that typed into `a`.
    let for_a: Vec<_> = changes
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == 0 || i % 2 == 1)
        .map(|(_, c)| c.clone())
        .collect();
    let mut fresh = AutoCommit::new();
    fresh.apply_changes(for_a).unwrap();
    let heads = fresh.get_heads();
    assert_eq!(fresh.get_missing_deps(&heads).len(), 19);
    // Each change names the one before it, whatever object that one touched, so only the first applies.
    let (_, id) = fresh.get(ROOT, "a").unwrap().unwrap();
    assert_eq!(fresh.text(&id).unwrap(), "a");
}

// -----------------------------------------------------------------------------------------------------------------
// Question 6: can the server look inside an update before applying it?
// -----------------------------------------------------------------------------------------------------------------

#[test]
fn what_an_update_shows_before_it_is_applied() {
    // Each client types one "Q" into a document that already holds some text.
    // yrs: an update decodes on its own, and its printed form shows what was inserted.
    {
        use yrs::updates::decoder::Decode;
        let (mut doc, _) = typed::<Yrs>(1, 10);
        let update = Yrs::insert(&mut doc, 3, "Q");
        let decoded = yrs::Update::decode_v1(&update).unwrap();
        assert!(format!("{decoded:?}").contains('Q'), "{decoded:?}");
    }
    // Automerge: an update is a change, which decodes on its own into its operations.
    {
        let (mut doc, _) = typed::<Am>(1, 10);
        let update = Am::insert(&mut doc, 3, "Q");
        let change = automerge::Change::from_bytes(update).unwrap();
        let ops = format!("{:?}", change.decode().operations);
        assert!(ops.contains('Q'), "{ops}");
    }
    // Loro: the bytes on their own give only counts and versions. To see the operations, the update is imported
    // into a scratch copy and the difference is exported as JSON.
    {
        let (mut doc, updates) = typed::<Loro>(1, 10);
        let mut server = Loro::new(100);
        for u in &updates {
            Loro::apply(&mut server, u).unwrap();
        }
        let update = Loro::insert(&mut doc, 3, "Q");
        let meta = loro::LoroDoc::decode_import_blob_meta(&update, true).unwrap();
        assert_eq!(meta.change_num, 1);
        let scratch = server.fork();
        let before = scratch.oplog_vv();
        scratch.import(&update).unwrap();
        let json = scratch
            .export_json_updates(&before, &scratch.oplog_vv())
            .unwrap();
        let ops: usize = json.changes.iter().map(|c| c.ops.len()).sum();
        assert_eq!(ops, 1);
        assert!(format!("{:?}", json.changes[0].ops[0].content).contains('Q'));
    }
}

#[test]
fn automerge_load_incremental_says_ok_to_bytes_it_cannot_read() {
    // The call a sync layer would reach for first reports success whatever it is given, and applies nothing. A
    // server has to decode the change itself to know an update was bad; the adapter does.
    let (_, updates) = typed::<Am>(1, 10);
    let mut server = Am::new(100);
    for u in &updates {
        Am::apply(&mut server, u).unwrap();
    }
    let before = Am::text(&mut server);
    for garbage in [
        &b"not an update"[..],
        &[0xff; 40],
        &updates[3][..updates[3].len() - 2],
    ] {
        assert!(server.load_incremental(garbage).is_ok());
        assert_eq!(Am::text(&mut server), before);
        assert!(Am::apply(&mut server, garbage).is_err());
    }
}

// -----------------------------------------------------------------------------------------------------------------
// Marks and undo
// -----------------------------------------------------------------------------------------------------------------

#[test]
fn overlapping_marks_from_two_clients_converge() {
    // Two clients hold "the quick brown fox". One makes characters 4 to 15 bold, the other 10 to 19 italic, at
    // the same time. Both must end with the same text and the same marks.
    // Loro
    {
        let (mut one, updates) = typed::<Loro>(1, 19);
        let mut two = Loro::new(2);
        for u in &updates {
            Loro::apply(&mut two, u).unwrap();
        }
        let mark = |doc: &loro::LoroDoc, range: std::ops::Range<usize>, key: &str| {
            let before = doc.oplog_vv();
            doc.get_text(TEXT).mark(range, key, true).unwrap();
            doc.commit();
            doc.export(loro::ExportMode::updates(&before)).unwrap()
        };
        let (u1, u2) = (mark(&one, 4..15, "bold"), mark(&two, 10..19, "italic"));
        Loro::apply(&mut one, &u2).unwrap();
        Loro::apply(&mut two, &u1).unwrap();
        let (d1, d2) = (one.get_text(TEXT).to_delta(), two.get_text(TEXT).to_delta());
        assert_eq!(format!("{d1:?}"), format!("{d2:?}"));
        assert_eq!(d1.len(), 4, "plain, bold, bold and italic, italic: {d1:?}");
    }
    // yrs
    {
        use std::sync::Arc;
        use yrs::types::Attrs;
        use yrs::{Any, Text, Transact};
        let (mut one, updates) = typed::<Yrs>(1, 19);
        let mut two = Yrs::new(2);
        for u in &updates {
            Yrs::apply(&mut two, u).unwrap();
        }
        let mark = |doc: &yrs::Doc, at: u32, len: u32, key: &str| {
            let text = doc.get_or_insert_text(TEXT);
            let mut txn = doc.transact_mut();
            text.format(
                &mut txn,
                at,
                len,
                Attrs::from([(Arc::from(key), Any::Bool(true))]),
            );
            txn.encode_update_v1()
        };
        let (u1, u2) = (mark(&one, 4, 11, "bold"), mark(&two, 10, 9, "italic"));
        Yrs::apply(&mut one, &u2).unwrap();
        Yrs::apply(&mut two, &u1).unwrap();
        let runs = |doc: &yrs::Doc| {
            let text = doc.get_or_insert_text(TEXT);
            let diff = text.diff(&doc.transact(), yrs::types::text::YChange::identity);
            diff.iter()
                .map(|d| {
                    let mut keys: Vec<String> = d
                        .attributes
                        .iter()
                        .flat_map(|a| a.keys().map(|k| k.to_string()))
                        .collect();
                    keys.sort();
                    format!("{:?}{keys:?}", d.insert)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(runs(&one), runs(&two));
        assert_eq!(runs(&one).len(), 4, "{:?}", runs(&one));
    }
    // Automerge
    {
        use automerge::marks::{ExpandMark, Mark};
        use automerge::transaction::Transactable;
        use automerge::{ROOT, ReadDoc};
        let (mut one, updates) = typed::<Am>(1, 19);
        let mut two = Am::new(2);
        for u in &updates {
            Am::apply(&mut two, u).unwrap();
        }
        let mark = |doc: &mut automerge::AutoCommit, start: usize, end: usize, key: &str| {
            let (_, id) = doc.get(ROOT, TEXT).unwrap().unwrap();
            Am::forget_unsent(doc);
            doc.mark(
                &id,
                Mark::new(key.to_string(), true, start, end),
                ExpandMark::None,
            )
            .unwrap();
            doc.save_incremental()
        };
        let (u1, u2) = (
            mark(&mut one, 4, 15, "bold"),
            mark(&mut two, 10, 19, "italic"),
        );
        Am::apply(&mut one, &u2).unwrap();
        Am::apply(&mut two, &u1).unwrap();
        let marks = |doc: &mut automerge::AutoCommit| {
            let (_, id) = doc.get(ROOT, TEXT).unwrap().unwrap();
            let mut m: Vec<String> = doc
                .marks(&id)
                .unwrap()
                .iter()
                .map(|m| format!("{} {}..{}", m.name(), m.start, m.end))
                .collect();
            m.sort();
            m
        };
        assert_eq!(marks(&mut one), marks(&mut two));
        assert_eq!(marks(&mut one), ["bold 4..15", "italic 10..19"]);
    }
}

#[test]
fn two_values_for_one_mark_end_as_one_value_everywhere() {
    // An exclusive mark: a stretch of text has one colour. Two clients colour overlapping stretches differently
    // at the same time. Both must end with the same colours, one per character.
    // Loro
    {
        let (mut one, updates) = typed::<Loro>(1, 19);
        let mut two = Loro::new(2);
        for u in &updates {
            Loro::apply(&mut two, u).unwrap();
        }
        // Loro wants a style declared before it is used; only a few names are known to it from the start.
        for doc in [&one, &two] {
            let mut styles = loro::StyleConfigMap::new();
            styles.insert("colour".into(), loro::StyleConfig::new());
            doc.config_text_style(styles);
        }
        let mark = |doc: &loro::LoroDoc, range: std::ops::Range<usize>, colour: &str| {
            let before = doc.oplog_vv();
            doc.get_text(TEXT).mark(range, "colour", colour).unwrap();
            doc.commit();
            doc.export(loro::ExportMode::updates(&before)).unwrap()
        };
        let (u1, u2) = (mark(&one, 4..15, "red"), mark(&two, 10..19, "blue"));
        Loro::apply(&mut one, &u2).unwrap();
        Loro::apply(&mut two, &u1).unwrap();
        let (d1, d2) = (
            format!("{:?}", one.get_text(TEXT).to_delta()),
            format!("{:?}", two.get_text(TEXT).to_delta()),
        );
        assert_eq!(d1, d2);
        assert!(d1.contains("red") && d1.contains("blue"), "{d1}");
        assert_eq!(
            one.get_text(TEXT).to_delta().len(),
            3,
            "plain, red, blue: {d1}"
        );
    }
    // yrs
    {
        use std::sync::Arc;
        use yrs::types::Attrs;
        use yrs::{Any, Text, Transact};
        let (mut one, updates) = typed::<Yrs>(1, 19);
        let mut two = Yrs::new(2);
        for u in &updates {
            Yrs::apply(&mut two, u).unwrap();
        }
        let mark = |doc: &yrs::Doc, at: u32, len: u32, colour: &str| {
            let text = doc.get_or_insert_text(TEXT);
            let mut txn = doc.transact_mut();
            let attrs = Attrs::from([(Arc::from("colour"), Any::from(colour))]);
            text.format(&mut txn, at, len, attrs);
            txn.encode_update_v1()
        };
        let (u1, u2) = (mark(&one, 4, 11, "red"), mark(&two, 10, 9, "blue"));
        Yrs::apply(&mut one, &u2).unwrap();
        Yrs::apply(&mut two, &u1).unwrap();
        let runs = |doc: &yrs::Doc| {
            let text = doc.get_or_insert_text(TEXT);
            let diff = text.diff(&doc.transact(), yrs::types::text::YChange::identity);
            diff.iter()
                .map(|d| {
                    format!(
                        "{:?} {:?}",
                        d.insert,
                        d.attributes.as_ref().and_then(|a| a.get("colour"))
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(runs(&one), runs(&two));
        // Both agree, but not on what either asked for: " fox", which only the second client coloured, ends
        // with no colour. The end of the first client's red stretch also ends the blue one.
        let got = runs(&one);
        assert_eq!(got.len(), 4, "{got:?}");
        assert!(
            got[1].contains("quick ") && got[1].contains("red"),
            "{got:?}"
        );
        assert!(
            got[2].contains("brown") && got[2].contains("blue"),
            "{got:?}"
        );
        assert!(
            got[3].contains(" fox") && got[3].ends_with("None"),
            "{got:?}"
        );
    }
    // Automerge
    {
        use automerge::marks::{ExpandMark, Mark};
        use automerge::transaction::Transactable;
        use automerge::{ROOT, ReadDoc};
        let (mut one, updates) = typed::<Am>(1, 19);
        let mut two = Am::new(2);
        for u in &updates {
            Am::apply(&mut two, u).unwrap();
        }
        let mark = |doc: &mut automerge::AutoCommit, start: usize, end: usize, colour: &str| {
            let (_, id) = doc.get(ROOT, TEXT).unwrap().unwrap();
            Am::forget_unsent(doc);
            doc.mark(
                &id,
                Mark::new("colour".to_string(), colour, start, end),
                ExpandMark::None,
            )
            .unwrap();
            doc.save_incremental()
        };
        let (u1, u2) = (mark(&mut one, 4, 15, "red"), mark(&mut two, 10, 19, "blue"));
        Am::apply(&mut one, &u2).unwrap();
        Am::apply(&mut two, &u1).unwrap();
        let marks = |doc: &mut automerge::AutoCommit| {
            let (_, id) = doc.get(ROOT, TEXT).unwrap().unwrap();
            let mut m: Vec<String> = doc
                .marks(&id)
                .unwrap()
                .iter()
                .map(|m| format!("{}..{} {}", m.start, m.end, m.value()))
                .collect();
            m.sort();
            m
        };
        assert_eq!(marks(&mut one), marks(&mut two));
        // Two stretches that do not overlap and together cover 4 to 19.
        assert_eq!(marks(&mut one).len(), 2, "{:?}", marks(&mut one));
    }
}

#[test]
fn undo_takes_back_my_edit_and_leaves_theirs() {
    // I type "mine" at the start; someone else types "theirs" at the end, and it reaches me. Then I undo.
    // Loro: the undo manager follows local changes only.
    {
        let (mut me, updates) = typed::<Loro>(1, 10);
        let mut them = Loro::new(2);
        for u in &updates {
            Loro::apply(&mut them, u).unwrap();
        }
        let mut undo = loro::UndoManager::new(&me);
        Loro::insert(&mut me, 0, "mine ");
        let theirs = Loro::insert(&mut them, 10, " theirs");
        Loro::apply(&mut me, &theirs).unwrap();
        assert_eq!(Loro::text(&mut me), "mine the quick  theirs");
        assert!(undo.undo().unwrap());
        assert_eq!(Loro::text(&mut me), "the quick  theirs");
    }
    // yrs: the same, when the remote update is applied in a transaction with an origin of its own. Applied in a
    // plain transaction, as a local edit is, the remote edit is taken for mine and undone with it.
    {
        use yrs::updates::decoder::Decode;
        use yrs::{Transact, UndoManager, Update};
        for (with_origin, after_undo) in [(true, "the quick  theirs"), (false, "the quick ")] {
            let (mut me, updates) = typed::<Yrs>(1, 10);
            let mut them = Yrs::new(2);
            for u in &updates {
                Yrs::apply(&mut them, u).unwrap();
            }
            let mut undo = UndoManager::new();
            undo.expand_scope(&me, &me.get_or_insert_text(TEXT));
            Yrs::insert(&mut me, 0, "mine ");
            let theirs = Update::decode_v1(&Yrs::insert(&mut them, 10, " theirs")).unwrap();
            if with_origin {
                me.transact_mut_with("remote").apply_update(theirs).unwrap();
            } else {
                me.transact_mut().apply_update(theirs).unwrap();
            }
            assert_eq!(Yrs::text(&mut me), "mine the quick  theirs");
            assert!(undo.undo_blocking());
            assert_eq!(
                Yrs::text(&mut me),
                after_undo,
                "origin given: {with_origin}"
            );
        }
    }
    // Automerge 0.12.0 has no undo manager; an application would build its own from inverse edits.
}
