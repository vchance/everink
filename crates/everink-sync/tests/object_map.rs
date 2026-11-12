//! Tests for the object map: convergence, the server's checks, the tree, order keys, and the canonical bytes.

mod common;

use std::collections::{HashMap, HashSet};

use common::{CAP, World, client_id, object_id, shuffle, stroke_body, stroke_points, xform};
use everink_sync::order::{MAX_ORDER_KEY_BYTES, OrderError};
use everink_sync::points::{MAX_STROKE_POINTS, PointsError};
use everink_sync::rng::SplitMix64;
use everink_sync::{
    Applied, At, Body, Change, ClientChangeId, DecodeError, Effect, FieldId, Kind, NewObject,
    Object, ObjectId, Op, OrderKey, Outcome, PackedPoints, Paint, Point, RejectCode, Replica, Room,
    Seq, UserId, Value,
};

fn new_replica(n: u8) -> Replica {
    Replica::new(client_id(n), UserId(u64::from(n)))
}

/// One `Applied` per effect, so that writes of one change can be delivered apart.
fn split(log: &[Applied]) -> (Vec<Applied>, Vec<Applied>) {
    let (mut creates, mut writes) = (Vec::new(), Vec::new());
    for a in log {
        for e in &a.effects {
            let one = Applied {
                seq: a.seq,
                client: a.client,
                change: a.change,
                effects: vec![e.clone()],
            };
            match e {
                Effect::Create(_) => creates.push(one),
                Effect::Write { .. } => writes.push(one),
            }
        }
    }
    (creates, writes)
}

// ---------------------------------------------------------------------------------------------------------------
// Convergence
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_random_run_exercises_every_path() {
    // The convergence tests below mean little if the random changes were all accepted or all alike.
    let mut w = World::new(1, 4);
    w.run(6_000);
    let rejected: HashSet<String> = w
        .tally
        .rejected
        .iter()
        .map(|c| format!("{c:?}").split('(').next().unwrap().to_string())
        .collect();
    for code in [
        "UnknownObject",
        "DuplicateRegister",
        "Invalid",
        "BadParent",
        "Cycle",
        "Conflict",
    ] {
        assert!(
            rejected.contains(code),
            "no change was rejected with {code}"
        );
    }
    println!(
        "random run: {} accepted, {} refused, {} objects",
        w.tally.accepted,
        w.tally.rejected.len(),
        w.room.len()
    );
    assert!(w.tally.accepted > 2_000, "accepted {}", w.tally.accepted);
    assert!(w.room.len() > 1_000);
    let nested = w
        .room
        .objects()
        .filter(|o| o.slot().parent.is_some())
        .count();
    let deleted = w.room.objects().filter(|o| !o.alive()).count();
    let rewritten = w
        .room
        .objects()
        .filter(|o| !o.later_stamps().is_empty())
        .count();
    assert!(nested > 100, "nested {nested}");
    assert!(deleted > 20, "deleted {deleted}");
    assert!(rewritten > 300, "rewritten {rewritten}");
    assert_eq!(w.room.head(), Seq(w.tally.accepted as u64));
    w.room.check_tree().unwrap();
}

#[test]
fn copies_converge_whatever_order_the_writes_arrive_in() {
    for seed in 0..20 {
        let mut w = World::new(100 + seed, 3);
        w.run(1_500);
        let want = w.room.encode_objects();

        // In order, as the server sent it.
        let mut in_order = new_replica(9);
        for a in &w.log {
            in_order.apply(a);
        }
        assert_eq!(in_order.encode_objects(), want, "seed {seed}: in order");

        // Objects first, then every write on its own in a random order, a third of them twice.
        let (creates, mut writes) = split(&w.log);
        let again: Vec<Applied> = writes.iter().step_by(3).cloned().collect();
        writes.extend(again);
        shuffle(&mut w.rng, &mut writes);
        let mut shuffled = new_replica(9);
        for a in creates.iter().chain(&writes) {
            shuffled.apply(a);
        }
        assert_eq!(shuffled.encode_objects(), want, "seed {seed}: shuffled");

        // Objects first, then the writes newest first: every write but the newest per register must be ignored.
        let mut reversed = new_replica(9);
        for a in creates.iter().chain(writes.iter().rev()) {
            reversed.apply(a);
        }
        assert_eq!(reversed.encode_objects(), want, "seed {seed}: reversed");
    }
}

#[test]
fn a_partial_copy_converges_from_overlapping_snapshots_and_stream() {
    for seed in 0..20 {
        let mut w = World::new(200 + seed, 3);
        let mut replica = new_replica(9);
        // The copy is told about a change only after the server accepted it, in order, as on one connection. It
        // also gets whole objects at random moments (an object coming into view), old messages again (a resend),
        // and now and then drops an object (it left the view) that may come back later.
        let mut delivered = 0usize;
        let mut held_since_enter: HashSet<ObjectId> = HashSet::new();
        for _ in 0..400 {
            w.run(5);
            // Deliver the stream up to now, minus a random tail that arrives next round.
            let upto = w.log.len() - w.rng.below(3).min(w.log.len() as u64) as usize;
            while delivered < upto {
                let a = &w.log[delivered];
                replica.apply(a);
                for e in &a.effects {
                    if let Effect::Create(o) = e {
                        held_since_enter.insert(o.id());
                    }
                }
                delivered += 1;
            }
            // Something old arrives again. A creation is left out when the copy no longer holds the object:
            // an object comes back into a copy only whole and current (the next step), never as an old creation.
            // The test after this one shows why.
            if delivered > 0 && w.rng.below(2) == 0 {
                let i = w.rng.below(delivered as u64) as usize;
                let mut old = w.log[i].clone();
                old.effects.retain(|e| match e {
                    Effect::Create(o) => replica.object(&o.id()).is_some(),
                    Effect::Write { .. } => true,
                });
                replica.apply(&old);
            }
            // An object leaves the view.
            if w.rng.below(3) == 0 && !held_since_enter.is_empty() {
                let id = *held_since_enter.iter().min().unwrap();
                replica.exit(&id);
                held_since_enter.remove(&id);
            }
            // Objects come into view: the server sends them whole, as they are now. The stream the copy has
            // not yet been given (the tail) will repeat some of the same writes: that is the overlap.
            for _ in 0..w.rng.below(4) {
                if w.ids.is_empty() {
                    break;
                }
                let id = w.ids[w.rng.below(w.ids.len() as u64) as usize];
                if let Some(o) = w.room.object(&id) {
                    replica.enter(o);
                    held_since_enter.insert(id);
                }
            }
        }
        while delivered < w.log.len() {
            replica.apply(&w.log[delivered]);
            delivered += 1;
        }
        // Every object the copy holds is byte for byte the room's.
        assert!(
            replica.len() > 100,
            "seed {seed}: the copy holds {}",
            replica.len()
        );
        assert!(
            replica.len() < w.room.len(),
            "seed {seed}: the copy is not partial"
        );
        for id in &w.ids {
            if let Some(mine) = replica.object(id) {
                let theirs = w.room.object(id).expect("the room has what a copy has");
                assert_eq!(mine.encode(), theirs.encode(), "seed {seed}");
            }
        }
    }
}

#[test]
fn an_object_must_come_back_into_a_copy_whole() {
    // A write to an object the copy does not hold is dropped. So when the object comes back, it has to come as
    // it is now. The server does that: an object entering the view is sent whole. This test pins the rule down
    // by showing what the other way would do.
    let mut d = Desk::new();
    let s = d.create(None, At::Front, false);
    let creation = Applied {
        seq: d.room.head(),
        client: client_id(0),
        change: ClientChangeId(0),
        effects: vec![Effect::Create(d.room.object(&s).unwrap().clone())],
    };
    let mut copy = new_replica(9);
    copy.apply(&creation);
    copy.exit(&s);
    // While the copy does not hold the object, it is renamed. The copy is told, and drops the write.
    let rename = d.accept(1, vec![set(s, Value::Name(Some("renamed".into())))]);
    copy.apply(&rename);
    assert!(copy.object(&s).is_none());

    // Right: the object comes back whole.
    let mut right = new_replica(9);
    right.enter(d.room.object(&s).unwrap());
    assert_eq!(right.object(&s).unwrap().name(), Some("renamed"));

    // Wrong: the old creation is replayed. The rename is gone, and nothing later brings it back.
    copy.apply(&creation);
    assert_eq!(copy.object(&s).unwrap().name(), None);
    // The whole object repairs it, whenever it arrives.
    copy.enter(d.room.object(&s).unwrap());
    assert_eq!(
        copy.object(&s).unwrap().encode(),
        d.room.object(&s).unwrap().encode()
    );
}

#[test]
fn the_room_agrees_with_a_plain_model_that_keeps_every_stamp() {
    // The model is written from the rule alone: replay the accepted changes in order; the last write to a
    // register is its value and its stamp. It keeps a stamp for every register, where an object keeps only the
    // later ones.
    let mut w = World::new(7, 4);
    w.run(8_000);
    let mut model: HashMap<(ObjectId, FieldId), (Seq, Value)> = HashMap::new();
    let mut created: HashMap<ObjectId, Seq> = HashMap::new();
    for a in &w.log {
        for e in &a.effects {
            match e {
                Effect::Create(o) => {
                    assert_eq!(o.created(), a.seq);
                    created.insert(o.id(), a.seq);
                    for f in FieldId::ALL {
                        if let Some(v) = o.value(f) {
                            model.insert((o.id(), f), (a.seq, v));
                        }
                    }
                }
                Effect::Write { id, value } => {
                    model.insert((*id, value.field()), (a.seq, value.clone()));
                }
            }
        }
    }
    assert_eq!(created.len(), w.room.len());
    let mut registers = 0;
    for o in w.room.objects() {
        for f in FieldId::ALL {
            match model.get(&(o.id(), f)) {
                Some((stamp, value)) => {
                    assert_eq!(o.value(f).as_ref(), Some(value));
                    assert_eq!(o.stamp(f), *stamp);
                    // The short form holds a stamp exactly for the registers written after creation.
                    let later = o.later_stamps().iter().any(|(lf, _)| *lf == f);
                    assert_eq!(later, *stamp > o.created());
                    registers += 1;
                }
                None => assert!(o.value(f).is_none()),
            }
        }
        // The ordinal counts creations in the order the room accepted them.
        assert!(o.ordinal().0 < w.room.len() as u32);
    }
    assert_eq!(registers, model.len());
    let ordinals: HashSet<u32> = w.room.objects().map(|o| o.ordinal().0).collect();
    assert_eq!(ordinals.len(), w.room.len(), "ordinals are not reused");
}

#[test]
fn a_change_sent_again_is_applied_once() {
    let mut w = World::new(11, 2);
    for _ in 0..600 {
        let client = w.rng.below(2) as usize;
        let change = w.random_change(client);
        let first = w.submit(client, &change);
        let before = w.room.encode_all();
        // Twice more, as a client does when it cannot tell whether the first copy arrived.
        for _ in 0..2 {
            let again = w
                .room
                .submit(client_id(client as u8), UserId(client as u64), &change);
            match (&first, &again) {
                (Outcome::Accepted(a), Outcome::Duplicate { highest, seq }) => {
                    assert_eq!((*highest, *seq), (change.id, a.seq));
                }
                // A rejected change is not remembered; sent again it is rejected again for the same reason.
                (Outcome::Rejected(a), Outcome::Rejected(b)) => assert_eq!(a, b),
                other => panic!("unexpected {other:?}"),
            }
            assert_eq!(w.room.encode_all(), before, "a resend changed the room");
        }
    }
    assert!(w.tally.accepted > 200);
}

#[test]
fn a_refused_change_leaves_the_room_untouched() {
    let mut w = World::new(13, 2);
    w.run(500);
    let groups = w.groups();
    let some = w
        .room
        .objects()
        .find(|o| o.kind() == Kind::Stroke)
        .unwrap()
        .id();
    // A change whose first operations are fine and whose last is not: nothing of it may be applied.
    let bad_ends: Vec<Op> = vec![
        Op::Set {
            id: object_id(&mut w.rng),
            value: Value::Alive(false),
            if_stamp: None,
        },
        Op::Set {
            id: some,
            value: Value::Clip(true),
            if_stamp: None,
        },
        Op::Set {
            id: some,
            value: Value::Name(Some("x".repeat(257).into())),
            if_stamp: None,
        },
        Op::Set {
            id: some,
            value: Value::Paint(Paint { rgba: 1 }),
            if_stamp: Some(Seq(u64::MAX)),
        },
        Op::SetSlot {
            id: groups[0],
            parent: Some(groups[0]),
            at: At::Front,
            if_stamp: None,
        },
        Op::SetSlot {
            id: some,
            parent: Some(some),
            at: At::Back,
            if_stamp: None,
        },
    ];
    for (refused, bad) in bad_ends.into_iter().enumerate() {
        let new_id = object_id(&mut w.rng);
        let change = Change {
            id: ClientChangeId(1_000_000 + refused as u64),
            observed: w.room.head(),
            ops: vec![
                Op::Create(NewObject {
                    id: new_id,
                    parent: None,
                    at: At::Front,
                    xform: xform(&mut w.rng),
                    name: None,
                    body: stroke_body(&mut w.rng, 10),
                }),
                Op::Set {
                    id: groups[1],
                    value: Value::Alive(false),
                    if_stamp: None,
                },
                bad,
            ],
        };
        let before = (w.room.encode_all(), w.room.head());
        let outcome = w.room.submit(client_id(0), UserId(0), &change);
        assert!(matches!(outcome, Outcome::Rejected(_)), "{outcome:?}");
        assert_eq!((w.room.encode_all(), w.room.head()), before);
        assert!(w.room.object(&new_id).is_none());
        w.room.check_tree().unwrap();
    }
    // The other whole-change refusals.
    let head = w.room.head();
    let submit = |room: &mut Room, id: u64, observed: Seq, ops: Vec<Op>| {
        room.submit(
            client_id(1),
            UserId(1),
            &Change {
                id: ClientChangeId(2_000_000 + id),
                observed,
                ops,
            },
        )
    };
    let set = |v: bool| Op::Set {
        id: some,
        value: Value::Alive(v),
        if_stamp: None,
    };
    assert_eq!(
        submit(&mut w.room, 1, head, vec![]),
        Outcome::Rejected(RejectCode::Empty)
    );
    assert_eq!(
        submit(&mut w.room, 2, Seq(head.0 + 1), vec![set(true)]),
        Outcome::Rejected(RejectCode::ObservedAhead)
    );
    assert_eq!(
        submit(&mut w.room, 3, head, vec![set(true), set(false)]),
        Outcome::Rejected(RejectCode::DuplicateRegister)
    );
    assert_eq!(
        submit(&mut w.room, 4, head, vec![set(true); 1_001]),
        Outcome::Rejected(RejectCode::TooManyOps)
    );
    let exists = Op::Create(NewObject {
        id: some,
        parent: None,
        at: At::Front,
        xform: xform(&mut w.rng),
        name: None,
        body: Body::Group { clip: false },
    });
    assert_eq!(
        submit(&mut w.room, 5, head, vec![exists]),
        Outcome::Rejected(RejectCode::IdExists)
    );
    assert_eq!(w.room.head(), head);
}

// ---------------------------------------------------------------------------------------------------------------
// The rules of section 02
// ---------------------------------------------------------------------------------------------------------------

struct Desk {
    room: Room,
    rng: SplitMix64,
    next: u64,
}

impl Desk {
    fn new() -> Desk {
        Desk {
            room: Room::new(CAP, 5),
            rng: SplitMix64(5),
            next: 1,
        }
    }

    fn change(&mut self, ops: Vec<Op>) -> Change {
        self.next += 1;
        Change {
            id: ClientChangeId(self.next),
            observed: self.room.head(),
            ops,
        }
    }

    fn send(&mut self, client: u8, change: &Change) -> Outcome {
        self.room
            .submit(client_id(client), UserId(u64::from(client)), change)
    }

    fn accept(&mut self, client: u8, ops: Vec<Op>) -> Applied {
        let change = self.change(ops);
        match self.send(client, &change) {
            Outcome::Accepted(a) => a,
            other => panic!("not accepted: {other:?}"),
        }
    }

    fn refuse(&mut self, client: u8, ops: Vec<Op>) -> RejectCode {
        let change = self.change(ops);
        match self.send(client, &change) {
            Outcome::Rejected(code) => code,
            other => panic!("not refused: {other:?}"),
        }
    }

    fn new_op(&mut self, id: ObjectId, parent: Option<ObjectId>, at: At, group: bool) -> Op {
        let body = if group {
            Body::Group { clip: false }
        } else {
            stroke_body(&mut self.rng, 5)
        };
        Op::Create(NewObject {
            id,
            parent,
            at,
            xform: xform(&mut self.rng),
            name: None,
            body,
        })
    }

    fn create(&mut self, parent: Option<ObjectId>, at: At, group: bool) -> ObjectId {
        let id = object_id(&mut self.rng);
        let op = self.new_op(id, parent, at, group);
        self.accept(0, vec![op]);
        id
    }
}

fn set(id: ObjectId, value: Value) -> Op {
    Op::Set {
        id,
        value,
        if_stamp: None,
    }
}

fn mv(id: ObjectId, parent: Option<ObjectId>) -> Op {
    Op::SetSlot {
        id,
        parent,
        at: At::Front,
        if_stamp: None,
    }
}

#[test]
fn delete_and_edit_commute_and_restore_brings_the_edit_back() {
    let mut d = Desk::new();
    let s = d.create(None, At::Front, false);
    d.accept(0, vec![set(s, Value::Alive(false))]);
    // An edit that arrives after the delete is applied to the hidden object.
    d.accept(1, vec![set(s, Value::Paint(Paint { rgba: 0xff00_00ff }))]);
    let o = d.room.object(&s).unwrap();
    assert!(!o.alive());
    assert_eq!(
        o.value(FieldId::Paint),
        Some(Value::Paint(Paint { rgba: 0xff00_00ff }))
    );
    d.accept(0, vec![set(s, Value::Alive(true))]);
    let o = d.room.object(&s).unwrap();
    assert!(o.alive());
    assert_eq!(
        o.value(FieldId::Paint),
        Some(Value::Paint(Paint { rgba: 0xff00_00ff }))
    );
}

#[test]
fn an_offline_change_wins_when_it_arrives() {
    // Owner decision 5: a change made offline is stamped when the server accepts it, so it wins over online
    // edits that were made after it but accepted before it.
    let mut d = Desk::new();
    let s = d.create(None, At::Front, false);
    let offline = d.change(vec![set(s, Value::Name(Some("offline".into())))]);
    // While that client is away, another renames the object, twice.
    d.accept(1, vec![set(s, Value::Name(Some("online 1".into())))]);
    d.accept(1, vec![set(s, Value::Name(Some("online 2".into())))]);
    let Outcome::Accepted(a) = d.send(2, &offline) else {
        panic!("the offline change was refused");
    };
    let o = d.room.object(&s).unwrap();
    assert_eq!(o.name(), Some("offline"));
    assert_eq!(o.stamp(FieldId::Name), a.seq);
    // A conditional write (undo) made against the old stamp is the one kind that does not win.
    let stale = o.created();
    let code = d.refuse(
        1,
        vec![Op::Set {
            id: s,
            value: Value::Name(None),
            if_stamp: Some(stale),
        }],
    );
    assert_eq!(code, RejectCode::Conflict);
}

#[test]
fn the_tree_stays_a_tree() {
    let mut d = Desk::new();
    let a = d.create(None, At::Front, true);
    let b = d.create(Some(a), At::Front, true);
    let c = d.create(Some(b), At::Front, true);
    let s = d.create(Some(c), At::Front, false);

    // Two people move at once: one puts a under c, the other is first and... each alone is fine, both are a loop.
    assert_eq!(d.refuse(0, vec![mv(a, Some(c))]), RejectCode::Cycle);
    assert_eq!(d.refuse(0, vec![mv(a, Some(a))]), RejectCode::Cycle);
    let first = d.change(vec![mv(c, None)]);
    let second = d.change(vec![mv(a, Some(c))]);
    assert!(matches!(d.send(1, &first), Outcome::Accepted(_)));
    // After the first, c is at the top, so a may go under it.
    assert!(matches!(d.send(2, &second), Outcome::Accepted(_)));
    // Now a is under c. The reverse move, made by someone who had not seen that, would close the loop.
    assert_eq!(d.refuse(0, vec![mv(c, Some(b))]), RejectCode::Cycle);
    d.room.check_tree().unwrap();

    // A stroke cannot be a parent.
    assert_eq!(d.refuse(0, vec![mv(a, Some(s))]), RejectCode::BadParent);
    // Nothing moves into a deleted group, and nothing is created in one; moving out is allowed.
    d.accept(0, vec![set(b, Value::Alive(false))]);
    let loose = d.create(None, At::Front, false);
    assert_eq!(d.refuse(0, vec![mv(loose, Some(b))]), RejectCode::BadParent);
    d.accept(0, vec![mv(b, None)]);
    d.room.check_tree().unwrap();

    // A group and its members in one change: the group is made first, and the members see it.
    let g = object_id(&mut d.rng);
    let make = d.new_op(g, None, At::Front, true);
    d.accept(0, vec![make, mv(s, Some(g)), mv(loose, Some(g))]);
    let kids: Vec<ObjectId> = d.room.children(Some(g)).collect();
    assert_eq!(
        kids,
        vec![s, loose],
        "each Front goes above the one before it"
    );
    d.room.check_tree().unwrap();
}

#[test]
fn concurrent_moves_never_make_a_cycle() {
    // Many clients move groups about, each from a view of the tree that is several changes old.
    for seed in 0..10 {
        let mut d = Desk::new();
        d.rng = SplitMix64(900 + seed);
        let groups: Vec<ObjectId> = (0..40).map(|_| d.create(None, At::Front, true)).collect();
        let (mut accepted, mut cycles) = (0, 0);
        for _ in 0..2_000 {
            let id = groups[d.rng.below(40) as usize];
            let parent = match d.rng.below(8) {
                0 => None,
                _ => Some(groups[d.rng.below(40) as usize]),
            };
            let change = d.change(vec![mv(id, parent)]);
            match d.send(0, &change) {
                Outcome::Accepted(_) => accepted += 1,
                Outcome::Rejected(RejectCode::Cycle) => cycles += 1,
                other => panic!("{other:?}"),
            }
            d.room.check_tree().unwrap();
        }
        assert!(
            accepted > 300 && cycles > 100,
            "accepted {accepted}, cycles {cycles}"
        );
    }
}

#[test]
fn front_and_back_are_resolved_against_every_sibling() {
    let mut d = Desk::new();
    let mut want: Vec<ObjectId> = Vec::new();
    for i in 0..300 {
        if i % 3 == 0 {
            want.insert(0, d.create(None, At::Back, false));
        } else {
            want.push(d.create(None, At::Front, false));
        }
    }
    assert_eq!(d.room.children(None).collect::<Vec<_>>(), want);
    // Between two neighbours.
    let key = |d: &Desk, id: &ObjectId| d.room.object(id).unwrap().slot().order.clone();
    let (below, above) = (key(&d, &want[100]), key(&d, &want[101]));
    let mid = d.create(
        None,
        At::Between(Some(below.clone()), Some(above.clone())),
        false,
    );
    want.insert(101, mid);
    assert_eq!(d.room.children(None).collect::<Vec<_>>(), want);
    // The wrong way round is refused.
    let id = object_id(&mut d.rng);
    let wrong = d.new_op(id, None, At::Between(Some(above), Some(below)), true);
    let code = d.refuse(0, vec![wrong]);
    assert_eq!(code, RejectCode::BadOrder);
}

// ---------------------------------------------------------------------------------------------------------------
// A client's own changes
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_client_sees_its_own_change_at_once_and_remote_writes_do_not_flicker_through() {
    let mut d = Desk::new();
    let s = d.create(None, At::Front, false);
    let mut me = new_replica(1);
    me.enter(d.room.object(&s).unwrap());
    let name = |r: &Replica| r.shown(&s).unwrap().name().map(str::to_string);

    // I rename the object. I see it before the server has answered.
    let mine = d.change(vec![set(s, Value::Name(Some("mine".into())))]);
    me.local(mine.clone());
    assert_eq!(name(&me).as_deref(), Some("mine"));
    assert_eq!(
        me.object(&s).unwrap().name(),
        None,
        "nothing is confirmed yet"
    );

    // Someone else's rename is accepted first and reaches me. It must not show: mine will be stamped later.
    let theirs = d.accept(2, vec![set(s, Value::Name(Some("theirs".into())))]);
    me.apply(&theirs);
    assert_eq!(name(&me).as_deref(), Some("mine"));
    assert_eq!(me.object(&s).unwrap().name(), Some("theirs"));

    // The server accepts mine; its answer replaces the pending change.
    let Outcome::Accepted(answer) = d.send(1, &mine) else {
        panic!()
    };
    me.apply(&answer);
    assert!(me.pending().is_empty());
    assert_eq!(name(&me).as_deref(), Some("mine"));
    assert_eq!(
        me.object(&s).unwrap().encode(),
        d.room.object(&s).unwrap().encode()
    );

    // A change of mine that the server refuses: the confirmed value shows again.
    let doomed = d.change(vec![Op::Set {
        id: s,
        value: Value::Name(Some("doomed".into())),
        if_stamp: Some(Seq(1)),
    }]);
    me.local(doomed.clone());
    assert_eq!(name(&me).as_deref(), Some("doomed"));
    assert!(matches!(
        d.send(1, &doomed),
        Outcome::Rejected(RejectCode::Conflict)
    ));
    me.rejected(doomed.id);
    assert_eq!(name(&me).as_deref(), Some("mine"));

    // A new object of mine shows before the server knows it.
    let id = object_id(&mut d.rng);
    let make = d.new_op(id, None, At::Front, false);
    let create = d.change(vec![make, set(s, Value::Name(Some("with it".into())))]);
    me.local(create.clone());
    assert!(me.shown(&id).is_some());
    assert!(me.object(&id).is_none());
    assert_eq!(name(&me).as_deref(), Some("with it"));
    let Outcome::Accepted(answer) = d.send(1, &create) else {
        panic!()
    };
    me.apply(&answer);
    assert_eq!(
        me.shown(&id).unwrap().encode(),
        d.room.object(&id).unwrap().encode()
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Order keys
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn a_key_between_two_keys_is_between_them() {
    let mut rng = SplitMix64(21);
    // Random insertion points: the list stays sorted and the keys stay short.
    let mut keys: Vec<OrderKey> = Vec::new();
    for _ in 0..10_000 {
        let gap = rng.below(keys.len() as u64 + 1) as usize;
        let below = gap.checked_sub(1).map(|i| &keys[i]);
        let above = keys.get(gap);
        let k = OrderKey::between(below, above, &mut rng).unwrap();
        assert!(
            below.is_none_or(|b| *b < k),
            "not above its lower neighbour"
        );
        assert!(
            above.is_none_or(|a| k < *a),
            "not below its upper neighbour"
        );
        assert_ne!(k.as_bytes().last(), Some(&0));
        keys.insert(gap, k);
    }
    assert!(keys.windows(2).all(|w| w[0] < w[1]));
    let longest = keys.iter().map(|k| k.as_bytes().len()).max().unwrap();
    assert!(
        longest <= 6,
        "10,000 random inserts gave a key of {longest} bytes"
    );

    // The edge cases of the digits.
    let k = |b: &[u8]| OrderKey::new(b).unwrap();
    for (a, b) in [
        (None, Some(k(&[1]))),
        (None, Some(k(&[0, 1]))),
        (None, Some(k(&[0, 0, 1]))),
        (Some(k(&[255])), None),
        (Some(k(&[255, 255, 255])), None),
        (Some(k(&[5])), Some(k(&[6]))),
        (Some(k(&[5])), Some(k(&[5, 1]))),
        (Some(k(&[5, 255])), Some(k(&[6]))),
        (Some(k(&[5, 255, 255])), Some(k(&[6]))),
        (Some(k(&[5])), Some(k(&[5, 0, 1]))),
        (Some(k(&[5, 254])), Some(k(&[5, 255]))),
    ] {
        for _ in 0..50 {
            let m = OrderKey::between(a.as_ref(), b.as_ref(), &mut rng).unwrap();
            assert!(a.as_ref().is_none_or(|a| *a < m), "{a:?} {m:?} {b:?}");
            assert!(b.as_ref().is_none_or(|b| m < *b), "{a:?} {m:?} {b:?}");
        }
    }
    assert_eq!(
        OrderKey::between(Some(&k(&[5])), Some(&k(&[5])), &mut rng),
        Err(OrderError::NotAscending)
    );
    assert_eq!(
        OrderKey::between(Some(&k(&[6])), Some(&k(&[5])), &mut rng),
        Err(OrderError::NotAscending)
    );
    assert_eq!(OrderKey::new(&[]), Err(OrderError::Empty));
    assert_eq!(OrderKey::new(&[3, 0]), Err(OrderError::EndsInZero));
    assert_eq!(OrderKey::new(&[1; 65]), Err(OrderError::TooLong));
    // Inline and heap keys compare as bytes, and a prefix sorts first.
    assert!(k(&[1; 14]) < k(&[1; 15]));
    assert!(k(&[1; 20]) < k(&[2]));
    assert_eq!(k(&[7; 15]), k(&[7; 15]));
}

#[test]
fn front_and_back_count_so_their_keys_stay_short() {
    let mut rng = SplitMix64(23);
    // 100,000 objects put in front one after another, and as many behind.
    let first = OrderKey::after(None, &mut rng).unwrap();
    let (mut top, mut bottom) = (first.clone(), first);
    let (mut longest_top, mut longest_bottom) = (0, 0);
    for _ in 0..100_000 {
        let up = OrderKey::after(Some(&top), &mut rng).unwrap();
        assert!(up > top);
        top = up;
        longest_top = longest_top.max(top.as_bytes().len());
        let down = OrderKey::before(Some(&bottom), &mut rng).unwrap();
        assert!(down < bottom);
        bottom = down;
        longest_bottom = longest_bottom.max(bottom.as_bytes().len());
    }
    assert_eq!((longest_top, longest_bottom), (4, 4));

    // From any key at all, including the ones `between` makes and ones that already look like counters:
    // the result is a valid key on the right side, and there is still room between the two.
    let mut longest = 0;
    for _ in 0..50_000 {
        let len = 1 + rng.below(12) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
        // Lean towards the first bytes that mean something to `after` and `before`.
        match rng.below(4) {
            0 => bytes[0] = 0xf8 + rng.below(8) as u8,
            1 => bytes[0] = rng.below(9) as u8,
            _ => {}
        }
        if rng.below(4) == 0 {
            bytes[1..].fill(if rng.below(2) == 0 { 0xff } else { 0 });
        }
        if *bytes.last().unwrap() == 0 {
            *bytes.last_mut().unwrap() = 1;
        }
        let k = OrderKey::new(&bytes).unwrap();
        let up = OrderKey::after(Some(&k), &mut rng).unwrap();
        let down = OrderKey::before(Some(&k), &mut rng).unwrap();
        assert!(down < k && k < up, "{down:?} {k:?} {up:?}");
        let mid = OrderKey::between(Some(&k), Some(&up), &mut rng).unwrap();
        assert!(k < mid && mid < up);
        let mid = OrderKey::between(Some(&down), Some(&k), &mut rng).unwrap();
        assert!(down < mid && mid < k);
        longest = longest.max(up.as_bytes().len()).max(down.as_bytes().len());
    }
    assert!(longest <= 13, "a key of {longest} bytes");
}

#[test]
fn always_inserting_at_one_end_grows_the_key_slowly_and_then_fails_cleanly() {
    // The worst case for `between`: every insert goes right below the last one. This is what the design's rule
    // does for every object put in front or behind, and why the room counts instead (the test above). A client
    // that always asks for "between the lowest and nothing" can still do it. The design has the server rewrite
    // keys past 24 bytes; the prototype does not, so this records how many inserts that takes.
    let mut rng = SplitMix64(22);
    let mut lowest = OrderKey::between(None, None, &mut rng).unwrap();
    let (mut inserts, mut at_24) = (0u32, None);
    loop {
        match OrderKey::between(None, Some(&lowest), &mut rng) {
            Ok(k) => {
                assert!(k < lowest);
                lowest = k;
                inserts += 1;
                if at_24.is_none() && lowest.as_bytes().len() > 24 {
                    at_24 = Some(inserts);
                }
            }
            Err(e) => {
                assert_eq!(e, OrderError::TooLong);
                break;
            }
        }
    }
    assert_eq!(lowest.as_bytes().len(), MAX_ORDER_KEY_BYTES);
    // About seven inserts fit in each byte: each takes the middle third of what is left below.
    let at_24 = at_24.unwrap();
    println!("at one end: past 24 bytes after {at_24} inserts, no key left after {inserts}");
    assert!(
        (100..400).contains(&at_24),
        "passed 24 bytes after {at_24} inserts"
    );
    assert!(
        (300..1_200).contains(&inserts),
        "full after {inserts} inserts"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Points and canonical bytes
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn points_pack_and_unpack_unchanged_and_bad_bytes_are_refused() {
    let mut rng = SplitMix64(31);
    for n in [0usize, 1, 2, 200, 20_000] {
        let points: Vec<Point> = (0..n)
            .map(|i| Point {
                // The far corners of the lattice, so that a difference needs all 33 bits.
                x: if i % 2 == 0 { i32::MIN } else { i32::MAX },
                y: rng.next_u64() as i32,
                pressure: i as u8,
            })
            .collect();
        let packed = PackedPoints::pack(&points).unwrap();
        assert_eq!(packed.count() as usize, n);
        assert_eq!(packed.to_vec(), points);
        assert_eq!(
            PackedPoints::from_bytes(packed.count(), packed.bytes()).unwrap(),
            packed
        );
    }
    let too_many = vec![
        Point {
            x: 0,
            y: 0,
            pressure: 0
        };
        MAX_STROKE_POINTS as usize + 1
    ];
    assert_eq!(PackedPoints::pack(&too_many), Err(PointsError::TooMany));

    let good = stroke_points(&mut rng, 50);
    let bytes = good.bytes();
    assert_eq!(
        PackedPoints::from_bytes(51, bytes),
        Err(PointsError::Truncated)
    );
    assert_eq!(
        PackedPoints::from_bytes(49, bytes),
        Err(PointsError::TrailingBytes)
    );
    assert_eq!(
        PackedPoints::from_bytes(MAX_STROKE_POINTS + 1, bytes),
        Err(PointsError::TooMany)
    );
    // A count far above what the bytes could hold is refused from the lengths alone.
    assert_eq!(
        PackedPoints::from_bytes(1, &[0; 12]),
        Err(PointsError::TrailingBytes)
    );
    // A difference written longer than it needs to be.
    assert_eq!(
        PackedPoints::from_bytes(1, &[0x80, 0x00, 0x00, 0x00]),
        Err(PointsError::NotCanonical)
    );
    // A point off the 32-bit lattice: i32::MAX, then one more.
    assert_eq!(
        PackedPoints::from_bytes(2, &[0xfe, 0xff, 0xff, 0xff, 0x0f, 0, 0, 0x02, 0, 0]),
        Err(PointsError::OutOfRange)
    );
    // Every cut-off of good bytes is refused, never a panic.
    for cut in 0..bytes.len() {
        assert!(PackedPoints::from_bytes(50, &bytes[..cut]).is_err());
    }
}

#[test]
fn the_bytes_of_a_state_are_canonical_and_bad_bytes_are_refused() {
    let mut w = World::new(41, 3);
    w.run(1_500);
    let bytes = w.room.encode_all();

    // Open, and write again: the same bytes, the same tree, the same order of children.
    let opened = Room::decode_all(&bytes, CAP, 1).unwrap();
    assert_eq!(opened.encode_all(), bytes);
    assert_eq!(opened.head(), w.room.head());
    opened.check_tree().unwrap();
    for g in w.groups().into_iter().map(Some).chain([None]) {
        assert_eq!(
            opened.children(g).collect::<Vec<_>>(),
            w.room.children(g).collect::<Vec<_>>()
        );
    }

    // One object at a time: every cut-off is refused, and a changed byte either is refused or decodes to an
    // object whose bytes are exactly the changed bytes (no two encodings of one value).
    let mut refused = 0;
    let mut accepted = 0;
    for o in w.room.objects().take(60) {
        let b = o.encode();
        assert_eq!(Object::decode(&b, &CAP).unwrap(), *o);
        for cut in 0..b.len() {
            assert!(Object::decode(&b[..cut], &CAP).is_err());
        }
        for _ in 0..200 {
            let mut m = b.clone();
            let i = w.rng.below(m.len() as u64) as usize;
            m[i] ^= 1 << w.rng.below(8);
            match Object::decode(&m, &CAP) {
                Ok(changed) => {
                    assert_eq!(changed.encode(), m);
                    accepted += 1;
                }
                Err(_) => refused += 1,
            }
        }
    }
    assert!(
        refused > 1_000 && accepted > 1_000,
        "refused {refused}, accepted {accepted}"
    );

    // Whole-state refusals.
    let mut extra = bytes.clone();
    extra.push(0);
    assert_eq!(
        Room::decode_all(&extra, CAP, 1).err(),
        Some(DecodeError::TrailingBytes)
    );
    assert!(Room::decode_all(&bytes[..bytes.len() - 1], CAP, 1).is_err());
    // A state whose objects form a loop is refused when it is opened.
    let mut d = Desk::new();
    let a = d.create(None, At::Front, true);
    let b = d.create(Some(a), At::Front, true);
    let (oa, ob) = (d.room.object(&a).unwrap(), d.room.object(&b).unwrap());
    let state = d.room.encode_all();
    let looped = {
        // Rewrite a's "no parent" flag as "parent b". The flag follows id, ordinal, creator, created, kind, alive.
        let a_bytes = oa.encode();
        let mut changed = a_bytes[..21].to_vec();
        assert_eq!(a_bytes[21], 0, "the flag is where the test thinks it is");
        changed.push(1);
        changed.extend_from_slice(&b.0);
        changed.extend_from_slice(&a_bytes[22..]);
        let b_bytes = ob.encode();
        let mut out = state[..3].to_vec();
        if a < b {
            out.extend_from_slice(&changed);
            out.extend_from_slice(&b_bytes);
        } else {
            out.extend_from_slice(&b_bytes);
            out.extend_from_slice(&changed);
        }
        out
    };
    assert_eq!(
        Room::decode_all(&looped, CAP, 1).err(),
        Some(DecodeError::Invalid("a tree that is not a tree"))
    );
}
