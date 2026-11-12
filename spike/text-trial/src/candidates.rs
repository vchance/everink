//! The three adapters.

use crate::{Candidate, TEXT};

// -----------------------------------------------------------------------------------------------------------------
// Loro
// -----------------------------------------------------------------------------------------------------------------

pub struct Loro;

impl Loro {
    fn edit(doc: &loro::LoroDoc, f: impl FnOnce(&loro::LoroText)) -> Vec<u8> {
        let before = doc.oplog_vv();
        f(&doc.get_text(TEXT));
        doc.commit();
        doc.export(loro::ExportMode::updates(&before))
            .expect("a document can export its own changes")
    }
}

impl Candidate for Loro {
    const NAME: &'static str = "loro 1.17.1";
    type Doc = loro::LoroDoc;

    fn new(client: u64) -> Self::Doc {
        let doc = loro::LoroDoc::new();
        doc.set_peer_id(client).expect("a peer id");
        doc
    }

    fn insert(doc: &mut Self::Doc, at: usize, text: &str) -> Vec<u8> {
        Loro::edit(doc, |t| {
            t.insert(at, text).expect("a position inside the text")
        })
    }

    fn delete(doc: &mut Self::Doc, at: usize, len: usize) -> Vec<u8> {
        Loro::edit(doc, |t| t.delete(at, len).expect("a range inside the text"))
    }

    fn apply(doc: &mut Self::Doc, update: &[u8]) -> Result<(), String> {
        doc.import(update).map(|_| ()).map_err(|e| e.to_string())
    }

    fn text(doc: &mut Self::Doc) -> String {
        doc.get_text(TEXT).to_string()
    }

    fn len(doc: &mut Self::Doc) -> usize {
        doc.get_text(TEXT).len_unicode()
    }

    fn snapshot(doc: &mut Self::Doc) -> Vec<u8> {
        doc.export(loro::ExportMode::Snapshot)
            .expect("a document can export itself")
    }

    fn load(client: u64, snapshot: &[u8]) -> Result<Self::Doc, String> {
        let doc = Loro::new(client);
        doc.import(snapshot).map_err(|e| e.to_string())?;
        Ok(doc)
    }

    fn scratch(doc: &mut Self::Doc) -> Self::Doc {
        doc.fork()
    }
}

// -----------------------------------------------------------------------------------------------------------------
// yrs
// -----------------------------------------------------------------------------------------------------------------

pub struct Yrs;

impl Candidate for Yrs {
    const NAME: &'static str = "yrs 0.28.1";
    type Doc = yrs::Doc;

    fn new(client: u64) -> Self::Doc {
        let doc = yrs::Doc::with_client_id(client);
        doc.get_or_insert_text(TEXT);
        doc
    }

    fn insert(doc: &mut Self::Doc, at: usize, text: &str) -> Vec<u8> {
        use yrs::{Text, Transact};
        let t = doc.get_or_insert_text(TEXT);
        let mut txn = doc.transact_mut();
        t.insert(&mut txn, at as u32, text);
        txn.encode_update_v1()
    }

    fn delete(doc: &mut Self::Doc, at: usize, len: usize) -> Vec<u8> {
        use yrs::{Text, Transact};
        let t = doc.get_or_insert_text(TEXT);
        let mut txn = doc.transact_mut();
        t.remove_range(&mut txn, at as u32, len as u32);
        txn.encode_update_v1()
    }

    fn apply(doc: &mut Self::Doc, update: &[u8]) -> Result<(), String> {
        use yrs::Transact;
        use yrs::updates::decoder::Decode;
        let update = yrs::Update::decode_v1(update).map_err(|e| e.to_string())?;
        doc.transact_mut()
            .apply_update(update)
            .map_err(|e| e.to_string())
    }

    fn text(doc: &mut Self::Doc) -> String {
        use yrs::{GetString, Transact};
        doc.get_or_insert_text(TEXT).get_string(&doc.transact())
    }

    fn len(doc: &mut Self::Doc) -> usize {
        // yrs counts in UTF-8 bytes by default; the trial's text is ASCII, where that is the character count.
        use yrs::{Text, Transact};
        doc.get_or_insert_text(TEXT).len(&doc.transact()) as usize
    }

    fn snapshot(doc: &mut Self::Doc) -> Vec<u8> {
        use yrs::{ReadTxn, Transact};
        doc.transact()
            .encode_state_as_update_v1(&yrs::StateVector::default())
    }

    fn load(client: u64, snapshot: &[u8]) -> Result<Self::Doc, String> {
        let mut doc = Yrs::new(client);
        Yrs::apply(&mut doc, snapshot)?;
        Ok(doc)
    }

    /// yrs has no fork: a scratch copy is a new document loaded from the whole state.
    fn scratch(doc: &mut Self::Doc) -> Self::Doc {
        let state = Yrs::snapshot(doc);
        Yrs::load(u64::from(u32::MAX), &state).expect("a document can load its own state")
    }
}

// -----------------------------------------------------------------------------------------------------------------
// Automerge
// -----------------------------------------------------------------------------------------------------------------

pub struct Am;

impl Am {
    /// In Automerge the text is an object that some change created, so every client must start from the same
    /// document that already holds it. This is that document, made the same way every time.
    fn base() -> Vec<u8> {
        use automerge::transaction::Transactable;
        let mut doc =
            automerge::AutoCommit::new().with_actor(automerge::ActorId::from(&[0u8; 16][..]));
        doc.put_object(automerge::ROOT, TEXT, automerge::ObjType::Text)
            .expect("a text object at the root");
        doc.save()
    }

    /// `save_incremental` gives every change since it was last called, the ones received from others included.
    /// Calling it before an edit and dropping the result leaves the edit's own change as the next thing it
    /// gives, so an update is always exactly one change.
    pub fn forget_unsent(doc: &mut automerge::AutoCommit) {
        doc.save_incremental();
    }

    fn text_id(doc: &automerge::AutoCommit) -> automerge::ObjId {
        use automerge::ReadDoc;
        doc.get(automerge::ROOT, TEXT)
            .ok()
            .flatten()
            .map(|(_, id)| id)
            .expect("the text object of the base document")
    }
}

impl Candidate for Am {
    const NAME: &'static str = "automerge 0.12.0";
    type Doc = automerge::AutoCommit;

    fn new(client: u64) -> Self::Doc {
        Am::load(client, &Am::base()).expect("the base document loads")
    }

    fn insert(doc: &mut Self::Doc, at: usize, text: &str) -> Vec<u8> {
        use automerge::transaction::Transactable;
        let id = Am::text_id(doc);
        Am::forget_unsent(doc);
        doc.splice_text(&id, at, 0, text)
            .expect("a position inside the text");
        doc.save_incremental()
    }

    fn delete(doc: &mut Self::Doc, at: usize, len: usize) -> Vec<u8> {
        use automerge::transaction::Transactable;
        let id = Am::text_id(doc);
        Am::forget_unsent(doc);
        doc.splice_text(&id, at, len as isize, "")
            .expect("a range inside the text");
        doc.save_incremental()
    }

    /// An update is one change. It is decoded here, which checks its checksum, and then applied.
    /// `load_incremental` would take the same bytes, but it answers "ok" to bytes it cannot read and applies
    /// nothing (see the test `automerge_load_incremental_says_ok_to_bytes_it_cannot_read`), so a server could
    /// not tell a bad update from a good one.
    fn apply(doc: &mut Self::Doc, update: &[u8]) -> Result<(), String> {
        let change = automerge::Change::from_bytes(update.to_vec()).map_err(|e| e.to_string())?;
        doc.apply_changes([change]).map_err(|e| e.to_string())
    }

    fn text(doc: &mut Self::Doc) -> String {
        use automerge::ReadDoc;
        let id = Am::text_id(doc);
        doc.text(&id).expect("the text")
    }

    fn len(doc: &mut Self::Doc) -> usize {
        use automerge::ReadDoc;
        let id = Am::text_id(doc);
        doc.length(&id)
    }

    fn snapshot(doc: &mut Self::Doc) -> Vec<u8> {
        doc.save()
    }

    fn load(client: u64, snapshot: &[u8]) -> Result<Self::Doc, String> {
        let mut doc = automerge::AutoCommit::load(snapshot).map_err(|e| e.to_string())?;
        doc.set_actor(automerge::ActorId::from(&client.to_be_bytes()[..]));
        // What was loaded is not something this client has to send.
        doc.save_incremental();
        Ok(doc)
    }

    fn scratch(doc: &mut Self::Doc) -> Self::Doc {
        doc.fork()
    }
}
