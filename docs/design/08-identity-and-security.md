# Identity, roles, limits and the threat model

Status: detailed design, 2026-11-11, proposal.

## Scope

This section says who a connection is, what it may do, how much of the server it may use, and what can go wrong.

It covers: the signed token and its keys; the WebSocket handshake and the session the server keeps; share links and
guests; the role and permission matrix; limits and rate limiting; tenants; blob access control and the security
headers of the blob store; threats from media and fonts; hardening of the web client; privacy, logging, retention
and the audit trail; supply-chain controls; the threat model; and the tests that keep denied paths denied.

It leaves to other sections, and only states the security requirements they must meet (see
[What this assumes of other sections](#what-this-assumes-of-other-sections)):

| Topic | Section |
|---|---|
| Position representation and the arithmetic on it (this section only sets the cap on depth) | 01 |
| Object and change types, the merge, undo | 02 |
| Wire messages, view subscriptions, server storage, versions | 03 |
| Presence routing and cursors | 04 |
| Image, video and font handling, the blob store interface | 05 |
| The accessibility mirror and text entry | 07 |

Terms used here. A **tenant** is one embedding app that uses an Everink server. A **surface** is one drawing space.
A **session** is the server's record of one authenticated connection. A **member** is someone who came in with a
signed token. A **guest** is someone who came in through a share link. The **operator** runs the server. A
**capability** is a value that grants access to whoever holds it, such as a share link.

## The problem

Everink ships a server (`everink-server`) that operators run, and clients that run on strangers' machines. The
server reads all content (owner decision 5), so the question is not "who may the server hide things from" but:

1. **Who is this connection?** There is no Everink account system, so identity comes from the embedding app
   (a signed token) or from nothing (a guest with a typed name).
2. **What may it do?** Four roles of member and guest, and about thirty operations (see the matrix).
3. **How much may it use?** A malicious client can send, per second, as much as its link allows. A 100 Mbit/s
   connection is about 12 MB/s. Without limits one client sending 1,000 strokes of 20,000 points each per second
   (about 240 MB/s of change data) would saturate the room, the log and every other client's downstream. The room
   applies each change to a spatial index, so cost is not only bytes: one position at the depth cap is about 3.1 KB
   and each comparison on it is long-number arithmetic.
4. **What if the thing is hostile?** Pasted files, uploaded fonts, display names, share links that leak, a stolen
   token, a compromised embedding app, a second embedding app on the same server.

The high-level design (`docs/design.md`, Security) lists the controls in one sentence each. This section turns each
into a mechanism with numbers, and finds two gaps in it (see "Corrections to the high-level design" under Design).

## What others do

Findings are from the pages cited under [Sources](#sources). Where a page did not say, the entry says "not stated"
and the claim is "unverified".

| System | Identity and room access | Notes for Everink |
|---|---|---|
| Liveblocks | The embedding app's server signs a JWT. Two kinds: an *access token* granting scopes per room (for example `room:write`, wildcard only as a suffix of a room id) and an *ID token* identifying the user, with permissions kept on the room (default, group and user levels). | Closest model to ours: the embedder is trusted for identity, the service enforces. Token lifetime and how the socket presents the token are not stated on the pages read (unverified). |
| tldraw sync | Leaves authentication and authorization to the integrator. Read-only is set per session when the socket is connected. A room id is in the URL; whether it is meant to be secret is not stated. | Shows the minimum: a per-session read-only flag. Our design adds tenant binding, expiry and limits. |
| PartyKit | Token in the query string of the WebSocket URL, checked in `onBeforeConnect`, which can reject with 401 before the room sees the connection. | Query-string tokens end up in access logs, which OWASP advises against (see below). We use a first message instead. |
| y-sweet | Server hands out per-document client tokens; the README says production apps must authenticate the user themselves. Expiry and read-only levels: not stated in the README (unverified; the docs page returned 404). | Same shape: embedder authenticates, service signs a narrow token. |
| Figma | Link sharing has audiences (anyone, organization, invited people only) and a view or edit role. Password and expiry only on paid plans. Whether an owner can switch a link off other than by restricting the audience: not stated. How its sockets authenticate: not researched (unverified). | Matches our "one role per link"; we add several links per surface, and expiry for all. |
| Google Drive | "Anyone" permission with a role; the link is a constant function of the file id, access checked against the ACL on every open; no expiry for `anyone` permissions; `allowFileDiscovery` hides the file from search. Resource keys exist (not read). | A link whose secret is the id cannot be revoked without changing the id. We store a hash of an independent secret, so each link is separately revocable. |
| Miro | Not researched (unverified). | |
| Excalidraw | Collaboration link `#room=<id>,<key>` puts an AES key after the `#`, so browsers do not send it to the server; the server is a relay of encrypted messages. The exact lengths in the link come from a fork's README (unverified). | We copy the fragment idea for the secret. We cannot copy the end-to-end encryption, because the server must read content (decision 5). |

Standards and guidance that shape the design:

- **W3C TAG, Good Practices for Capability URLs.** At least 120 bits of entropy from a secure generator; `Referer`
  leaks to third-party links; a secret in the fragment stays out of `Referer` but scripts on the page can read it
  and history keeps it; make URLs expire; give the sharer a way to revoke; issue several URLs per resource so one
  can be revoked alone; serve over HTTPS; a GET must not cause side effects.
- **RFC 8725 (JWT best current practice).** The verifier fixes the allowed algorithms (3.1); validate issuer and
  subject (3.8) and audience (3.9); do not trust `kid`, `jku`, `x5u` (3.10); use explicit `typ` (3.11); make
  different kinds of JWT mutually exclusive by `typ`, key or audience (3.12). Threats include `none` and algorithm
  confusion (2.1), substitution (2.7) and cross-JWT confusion (2.8).
- **OWASP WebSocket Security Cheat Sheet.** Check `Origin` on every handshake against an allow-list; browsers send
  cookies on the handshake, so cookie authentication invites cross-site WebSocket hijacking (CSWSH); do not put
  tokens in the URL; for browser clients send the token in the first message, accept nothing else until it is
  checked, close on failure or timeout, limit unauthenticated connections; cap message size (64 KB or less is the
  page's figure); rate-limit.
- **RFC 6455.** The `Origin` header lets a server refuse connections from unexpected web pages; non-browser clients
  can send any value, so it protects only against browsers.
- **OWASP File Upload Cheat Sheet.** Allow-list types; do not trust `Content-Type`; generate names on the server;
  cap sizes; serve from a different host; re-encode images and strip metadata (not a guarantee); sandbox the
  processing; defense in depth. The page does not cover polyglots or SVG (those rules below are ours).
- **OWASP ASVS 5.0.0** (published 2025-05-30), 17 chapters, V1 Encoding and Sanitization to V17 WebRTC. This
  section maps to V2, V3, V4, V5, V6, V7, V8, V9, V11, V12, V13, V14, V15 and V16.

## Options considered

Three choices need a comparison: the token format, how the token reaches the server, and the rate-limit algorithm.

### Token format

| | JWT (RFC 7519) profile, EdDSA only | PASETO v4.public | Biscuit |
|---|---|---|---|
| Signature | Ed25519 (RFC 8037) by our profile; the format allows many algorithms, the verifier allows one | Ed25519, fixed by the version; no choice at all | Ed25519; chain of signed blocks |
| Algorithm confusion | Possible in the format, prevented by a one-algorithm allow-list and `typ` | Not possible by design | Not possible by design |
| Embedders can mint it in | Every language and every identity library | Libraries exist for many languages; fewer than JWT (not counted) | Rust, Haskell and others listed by the project; far fewer |
| Rust verifier | `jsonwebtoken` 11.1.0, MIT, widely used | `pasetors` 0.8.1, MIT, 10.5 million downloads | `biscuit-auth` 7.0.0-alpha.1 (pre-release), Apache-2.0 |
| Extra power | None needed | None needed | Offline narrowing of rights by the holder; Datalog policies |
| Cost | Must be used carefully; we write the profile | Smaller ecosystem for embedders | A policy language the embedder must learn; we need no delegation |
| Audit status of the library | Not found (unverified) | Not mentioned in its docs (unverified) | Not researched (unverified) |

Choice: **JWT with a strict profile, Ed25519 only.** Reason: the embedder is a third party we do not control, and
the format every identity product and every language can already produce is worth more than PASETO's removal of
algorithm choice, which our profile also removes on our side (one algorithm, one `typ`, claims checked by a function
that fails closed). Given up: a smaller attack surface in the format itself, and the embedder's convenience when its
language lacks Ed25519 (RFC 8037 EdDSA is less common in older libraries than RS256 or ES256; unverified for any
particular library). Biscuit is rejected because nothing in the design needs holder-side attenuation; a guest link
already narrows rights on the server.

### How the token reaches the server

| | Query string | Cookie | `Sec-WebSocket-Protocol` | First message after upgrade |
|---|---|---|---|---|
| Visible in logs and proxies | Yes, by default | No | Often logged, as a header | No |
| CSWSH | No | Yes, browsers attach cookies | No | No |
| Works for native clients | Yes | Needs a cookie jar | Yes | Yes |
| Server can refuse before accepting the socket | Yes | Yes | Yes | No; costs a short-lived unauthenticated socket |
| Standard practice | Common (PartyKit) | Common | A hack; the value is meant for a protocol name | The OWASP recommendation |

Choice: **first message.** Given up: the server holds an unauthenticated socket for up to 5 seconds, which the
limits below bound.

### Rate-limit algorithm

| | Fixed window | Token bucket | GCRA (generic cell rate algorithm) |
|---|---|---|---|
| State per key | counter and window start | tokens and last time | one timestamp |
| Allows boundary bursts of 2x | Yes | No | No |
| Weighted cost (a 100 KB message costs more) | Yes | Yes | Yes |
| Rust crate | many | `governor` | `governor` (its algorithm is GCRA; the crate lists "gcra" as a keyword, its description does not say so; unverified) |

Choice: **GCRA through `governor` 0.10.4 (MIT).** It behaves like a token bucket with a burst size and a rate, in
one timestamp per key, which keeps per-address tables small. Given up: nothing that matters here; fixed windows are
rejected for the 2x burst at a window edge.

## Design

### Corrections to the high-level design

1. **"Anyone with the editor role can save a named version" and a restore needs an author.** A guest with an editor
   link has the editor role. Versions are the stated remedy for defacement, so a guest must not be able to create,
   delete or restore them. Only members create, delete or restore versions. Until the owner decides
   (question 10), restore is owner-only.
2. **Defacement remedy.** The high-level design offers versions only. A restore also throws away honest work done
   since. Every object records who created it (high-level Objects section), so this design adds two owner actions:
   *delete everything created by this guest or link* (section 02's `Op::DeleteBy`: the server expands it into one
   change of its own, which can be restored) and *freeze guests* (guests become viewers at once, without turning
   links off). Versions remain the last resort.
3. **A hash in a change must not be a read permission.** Blobs are named by content hash and deduplicated within a
   tenant. If a change could say "image with hash H" for any H that exists in the tenant, a user of one surface who
   learned H could read a file from a surface they cannot see. The rule below ties each blob to the surfaces that
   reference it, by upload or by a server-side copy.
4. **"Managing members" does not exist.** Everink has no member list: roles come from the embedder's tokens on every
   connect. The operation is removed from the matrix and replaced by the embedder issuing a different token.
5. **Ink and presence for viewers.** Viewers send cursor and view (requirement 7 needs it; views travel as sync
   messages, presence carries the cursor) but may not send live ink.

### Types

Sketches only; names are proposals.

```rust
/// Chosen by the operator. Lowercase text: ASCII letters, digits and '-', 1 to 32 bytes. Every section uses this type.
struct TenantId(String);
struct SurfaceId(String);          // opaque, 1..=64 bytes, [A-Za-z0-9_-]
struct UserSub(String);            // opaque from the embedder, 1..=128 bytes UTF-8, no control characters

/// Every storage, blob, room and audit call takes a TenantScope. There is no lookup by SurfaceId alone.
struct TenantScope { tenant: TenantId }
struct RoomKey { tenant: TenantId, surface: SurfaceId }

enum Role { Viewer, Editor, Owner }  // ordered; guests never get Owner

enum Principal {
    Member { sub: UserSub },
    Guest { guest_id: GuestId, link: LinkId },   // GuestId: 128 random bits assigned at exchange
}

struct Session {
    id: SessionId,                 // 128 random bits, never reused
    room: RoomKey,
    principal: Principal,
    role: Role,
    display_name: DisplayName,     // already validated
    locatable: bool,               // may others jump to or follow this person
    via: Via,
    started: Instant,
    expires: Instant,              // see "Lifetimes"
    budgets: SessionBudgets,       // rate limiters and the write budget
}

enum Via {
    Token { kid: KeyId, jti: Jti, token_iat: u64 },
    Link { link: LinkId },
}

/// The only way code outside the auth module can ask a question about rights.
struct Authz;
impl Authz {
    fn check(session: &Session, op: &Operation, surface: &SurfaceState) -> Result<(), Denied>;
}
```

`Operation` is an enum with one variant per row of the matrix. `check` is a total `match` with no wildcard arm, so
adding an operation without deciding its rights does not compile.

### The token

A **connect token** is a compact JWS (three base64url parts) with this profile.

Header (all three required; any other header member, including `jku`, `jwk`, `x5u`, `x5c`, `crit`, is a rejection):

| Field | Value | Why |
|---|---|---|
| `alg` | `EdDSA` only (Ed25519). The verifier is called with exactly this algorithm. | RFC 8725 3.1; stops `none` and key confusion |
| `typ` | `everink-connect+jwt` | RFC 8725 3.11 and 3.12: a token of another kind, or from another system sharing the key, is refused |
| `kid` | Key id, 1..=32 bytes `[A-Za-z0-9_-]`, looked up only in the tenant's configured keys | RFC 8725 3.10: never fetched, never used as a path or query |

Payload (JSON object, at most 2 KiB after decoding, duplicate keys rejected, UTF-8 only):

| Claim | Type | Required | Rule |
|---|---|---|---|
| `iss` | string | yes | Must equal the tenant id taken from the connection path. Chooses the key set. |
| `aud` | string | yes | Must equal the server's configured audience, `everink:<deployment id>`. A staging token fails in production. |
| `sub` | string | yes | The user, stable per person in the embedder, 1..=128 bytes. Identity inside Everink is `(tenant, sub)`. |
| `surface` | string | yes | Must equal the surface id from the path. A token opens exactly one surface. |
| `role` | `owner`, `editor` or `viewer` | yes | Any other value rejects the token. |
| `name` | string | yes | Display name; sanitized, never rejected (rules below). |
| `iat` | integer seconds | yes | Not more than 60 s in the future. |
| `exp` | integer seconds | yes | `exp - iat <= 600`. The verifier rejects longer lives, so a configuration mistake cannot mint a year-long token. 30 s of clock leeway. |
| `jti` | string | yes | 16..=64 bytes, random. Logged and audited; used for revocation and, if the tenant turns it on, single use. |
| `loc` | bool | no | Default for "others may jump to me" (default true). The person can still change it. |

Unknown claims are ignored and never trusted. Colors are chosen by the server, not by a claim, so a token cannot
claim someone else's color.

**Lifetime.** The embedder should issue tokens of 5 minutes. The server accepts up to 10 minutes. A token may be
presented again within its life, which makes reconnects cheap: a server restart does not send every client back to
the embedder at once. A tenant can turn on single-use (`jti` kept in memory for the token's remaining life), at the
cost of one embedder round trip per connect.

**Keys.**

- Each tenant has between 1 and 4 public keys, each with an id and optional `not_before` and `not_after` times. The
  server holds public keys only. The embedder holds the private key and nothing else is secret about verification.
- Distribution is by **configured keys** (operator configuration, or the tenant admin interface for its own keys) by
  default, because it needs no outbound network call from the server and nothing for an attacker to redirect. A
  tenant may instead give one **JWKS URL** (RFC 7517 key set). The server then fetches it only from that
  operator-configured HTTPS address, never from a token; it refuses redirects, private-network addresses (unless the
  operator allows them), bodies over 64 KiB and responses slower than 5 s; it caches for 5 minutes, keeps a stale
  copy for 24 hours when the fetch fails, and fetches again for an unknown `kid` at most once a minute per tenant.
  Choice: configured keys first, JWKS optional, because an unreachable or hijacked JWKS address is a new way to deny
  service or to inject a key.
- **Rotation** is overlap: add the new key; start signing with it; wait one maximum token life (10 minutes) after
  the last token under the old key; remove the old key. Sessions record their `kid`; removing a key ends the
  sessions that came in under it at their next refresh, or at once if the operator uses the emergency form.
- **Compromise** of a tenant key: remove it, and set the tenant's *revocation floor* to now (below). The attacker
  can have minted tokens only for that tenant's surfaces.

**Revocation.** Tokens are short, so most revocation is expiry. Three faster tools:

1. *Revocation floor* per tenant (tenant admin), optionally per `sub` or per surface: tokens with `iat` before the
   floor are refused and sessions that entered on such a token are closed. Propagation to rooms on other machines:
   at most 5 s.
2. *Remove a key* (tenant admin for its connect keys, operator for any), as above.
3. *Suspend the tenant* (operator): all its sessions close and new ones are refused.

**The admin interface is two interfaces,** so that a tenant can never change the limits the operator set on it.

| | Operator admin | Tenant admin |
|---|---|---|
| Who | The party running the server | One embedding app, for its own tenant only |
| Operations | Register a tenant and its keys; set tenant quotas and ceilings; suspend or resume a tenant; delete a tenant; set the tenant admin key list; read any audit trail | Set its revocation floor; add or remove its own connect keys, up to the operator's limit (4); delete a surface; erase a user; read its own audit trail |
| Key list | Operator admin keys, in the server's own configuration, not per tenant | Up to 4 tenant admin keys per tenant, set by the operator only, so a stolen tenant admin key cannot lock the tenant out of recovery |
| Token `typ` | `everink-operator+jwt` | `everink-tenant-admin+jwt` |

A tenant admin token that names a quota, a ceiling or another tenant is refused. Nothing a tenant sends can raise a
quota that applies to it.

Both admin tokens use the same profile as the connect token, with the differences below. They are checked by the
same function, so `alg`, `kid`, header and size rules (RFC 8725 3.1, 3.10 to 3.12) are identical.

| Field | Value |
|---|---|
| Header `alg` | `EdDSA` only. Any other header member (`jku`, `jwk`, `x5u`, `x5c`, `crit`) rejects the token. |
| Header `typ` | `everink-operator+jwt` or `everink-tenant-admin+jwt`; each endpoint accepts exactly one. A connect token, an operator token and a tenant admin token cannot be swapped. |
| Header `kid` | Looked up only in that interface's key list. |
| `iss` | The tenant id for a tenant admin token; the fixed string `operator` for an operator token. |
| `aud` | `everink-admin:<deployment id>`, so a token for another deployment fails. |
| `sub` | The administrator's identity, 1 to 128 bytes. Written to the audit trail. |
| `op` | Exactly one operation name from the table above. The token is good for that operation only. |
| `tgt` | The target of the operation (surface id, `sub`, key id), where the operation has one. Must match the request. |
| `iat`, `exp` | `exp - iat <= 60`; 10 s of clock leeway. |
| `jti` | 128 random bits. **Single use:** the server keeps each `jti` until `exp` plus leeway in its shared store (the same store as the revocation floor), not in a room's memory, so a replay is refused after a restart and on another node. A connect token's `jti`, by contrast, is kept in the room's memory and only when the tenant turns single use on. |

**Refresh in a long session.** A session ends when its token expires unless refreshed. At half the token's life the
client asks the embedder for a new token and sends `refresh { token }` on the open socket. The server runs the same
checks and also requires the same `iss`, `sub` and `surface` as the session; it then replaces role, name and expiry.
A lower role applies to the next message, and one that loses upload rights raises the session's upload floor
(below). A higher role applies too, which is how the embedder promotes someone. The server sends `refresh_needed` 2
minutes before expiry. If none arrives, it closes with code 4401, the client fetches a new token and reconnects, and
queued changes merge on reconnect. If the embedder is down for longer than a token's life, sessions end: the design
fails closed.

### The connection handshake

The WebSocket address is `wss://<server>/v1/t/<tenant>/s/<surface>/ws`. The tenant and surface are in the path
because they are not secrets and the server needs them to choose keys and limits. Browsers cannot set headers on a
WebSocket request, so the credential travels in the first message.

1. **Network checks before the upgrade.** TLS 1.2 or later (1.3 preferred). The request line and headers must arrive
   within 10 s and be under 8 KiB. The server looks up the tenant and applies the **per-address pre-authentication
   limit**: at most 64 sockets not yet authenticated per address (per /64 for IPv6) and 20 new sockets per minute
   per address. Unknown tenant, unknown surface and a suspended tenant all get the same refusal, so the response
   does not say which surfaces exist.
2. **Origin check.** If an `Origin` header is present it must be an exact string match (no wildcard, no substring)
   with one of the tenant's configured origins; otherwise `403`. If it is absent, the tenant's `allow_no_origin`
   setting decides (default true, because native shells send none). The check is defense in depth: Everink uses
   no cookies, so a hostile web page has no ambient credential to ride (this is what stops CSWSH), and it can only
   spend the victim's address on pre-authentication sockets. It also keeps an unrelated site from using a tenant's
   server as free relay.
3. **Upgrade.** `permessage-deflate` is not negotiated (see Limits). The sub-protocol is `everink.v1` (section 03).
   The server sends nothing. This one WebSocket carries document changes, sync messages and presence as separate
   message classes; the authentication frame comes before all of them, and the rate limits below are per class.
4. **Authentication frame.** Within 5 s the client must send one binary frame of at most 4 KiB, either
   `auth { token }` (members) or `auth { ticket }` (guests, from the link exchange). Anything else, a late frame, a
   larger frame or a second frame before the answer closes the socket (code 4400 or 4408).
5. **Verification (member).** Parse the header; reject unless `alg`, `typ` and `kid` are as profiled; find the key
   in the tenant's set and check its time window; verify the Ed25519 signature; then check `iss`, `aud`, `surface`,
   `exp`, `iat`, `role`, claim sizes and the revocation floor, in that order, failing closed on the first error. The
   reply to every failure is the same code, 4401, with no reason (the reason goes to the log).
6. **Session creation.** The server sanitizes the name, takes the role from the token, counts the user's other
   sessions (at most 8 per `(tenant, sub, surface)`), counts the surface's sessions against its cap, creates the
   `Session`, picks `expires = min(token.exp, now + 15 min)`, and replies `auth_ok { session_id, expires, role }`.
7. **After `auth_ok`** the room accepts the normal messages (section 03). Every message is checked by `Authz::check`
   against the session as it is *now*, not as it was at connect.

**Upload requests** (section 05) are not authenticated by the connection. A client asks for an **upload grant** over
the socket (`UploadGrantRequest { req, blob, size, kind }`, section 03) and presents it on every upload request. The
grant is defined under Blob access control below and is the only upload credential. HTTP endpoints take credentials
in an `Authorization` header on `fetch` requests made with `credentials: omit`; because that is a header and not a
cookie, cross-site request forgery has nothing to forge. CORS on these endpoints allows only the tenant's origins.

**The session the server keeps** lives in memory in the surface's room, not in a database. A restart ends all
sessions; clients reconnect with the token (still valid for up to 10 minutes) or fetch another. Guest tickets are
the exception, kept in the room's durable settings (below).

### Share links

A **share link** lets a person without an account join a surface as a guest.

**Creation** (owner members only; the audit trail records who and when):

```rust
struct ShareLink {
    id: LinkId,                 // 64 random bits, public: shown in lists and logs, grants nothing
    secret_hash: [u8; 32],      // SHA-256 of the 256-bit secret; the secret itself is never stored
    role: LinkRole,             // Viewer or Editor; Owner is not representable
    created_by: UserSub,
    created: Timestamp,
    expires: Option<Timestamp>, // default 30 days; "never" only if the tenant allows it
    max_guests: u16,            // concurrent, default 100
    state: LinkState,           // Active, Off { by, at }, Expired
    label: String,              // owner's note, 1..=60 chars
}
```

- The secret is **256 bits** from the operating system's random generator, written as 43 base64url characters. The
  TAG asks for at least 120 bits; 256 costs nothing and leaves room for any future truncation. A plain SHA-256 of
  the secret is enough for storage because the secret has full entropy: an attacker who reads the table cannot
  search for it, and a slow password hash would only slow every legitimate join.
- A surface has at most **5 active links**, at most one per owner-chosen label, with a role each. Several links let
  an owner give a link to each group and turn one off without disturbing the rest (TAG advice). A tenant may lower
  the maximum, or forbid links altogether (`guest_links: off`) or cap their role at viewer.
- The link text is `<embedder page>#join=<secret>`. The server never sees the whole address. The page is the
  embedder's, and the web shell provides one function, `join_with_link`, which does steps 1 to 3 below.

**Exchange** (all in the first second of the visit):

1. The page script reads `location.hash`, copies the secret into a local variable and calls
   `history.replaceState` to remove the fragment, so a later bookmark, copy of the address bar or in-page error
   report does not carry it.
2. It asks the person for a name and sends `POST /v1/t/<tenant>/s/<surface>/join` with `{secret, name}` as JSON over
   HTTPS, `credentials: omit`.
3. The server hashes the secret, finds an active, unexpired link on that surface (a hash from another surface
   finds nothing), validates the name, applies the guest quotas, and returns `{ticket, expires}`. The ticket is an
   opaque 192-bit random value; the server stores its hash with the `GuestId`, the `LinkId`, the name and the
   times.
4. The client opens the WebSocket and sends `auth { ticket }`. The session role is the link's role.

The typed name travels in this exchange and nowhere in presence (section 04 drops it from the presence hello). The
secret is used once per visitor; after that only the ticket is used. The ticket is kept in `sessionStorage` on the
web (gone when the tab closes) and in memory on native shells. The exchange endpoint is rate-limited at 10 failed
attempts per minute per address and 100 per minute per surface, and every failure returns the same answer whether
the link never existed, is off or has expired (a guess learns nothing; at 256 bits a guess cannot succeed in any
case). The secret is accepted only in the body of a `POST`, never in a `GET` (TAG: a `GET` has no side effects, and
`GET` addresses are logged).

**Guest session lifetime.** A ticket lasts at most 24 hours from the exchange, and is dead after 2 hours with no
connection. A guest who returns after that re-enters the link and types a name again. Within the 24 hours the
guest may reconnect with the ticket and keep the same `GuestId`, which keeps their authorship stable. Leaving the
session (a visible "leave" action, or the tab closing for `sessionStorage`) clears the ticket and the local cache.

**Turning a link off.** The owner sets the state to Off. In the same step the room closes every session with
`Via::Link(id)` using code 4403, deletes their tickets, and blocks the exchange. Because the link state lives
with the room that owns the surface, this takes under a second on a single room. The tenant admin
interface can do the same. Expiry does the same at the expiry time (a timer in the room). Changing a link's role
applies to open sessions at once.

**Guest names.** A guest name is a label. The rules, applied on the exchange and on every rename:

| Rule | Detail |
|---|---|
| Normalization | Unicode NFC (`unicode-normalization`), then trim, then collapse runs of white space to one space. |
| Length | 1 to 40 grapheme clusters after normalization (counted with `unicode-segmentation`), and at most 160 bytes. |
| Allowed | Letters, marks, numbers, punctuation, symbols (so emoji work) and the ASCII space. |
| Refused | Control characters, format characters (this removes bidirectional overrides and zero-width characters), line and paragraph separators, private-use and unassigned code points; more than 3 combining marks in a row. |
| Impersonation | Compute the confusable skeleton of the name (UTS 39, `unicode-security`) in lower case. Refuse if it equals the skeleton of a member's name currently on the surface, or one that has been on it in the last 24 hours. The message says "that name is in use". |
| Duplicates among guests | Allowed; the later one gets " (2)" appended. |
| Mark | The banner draws a *Guest* badge and the first-letter avatar in a guest style. The mark is drawn by the client from the server's `kind` field, not from the name, so a name cannot imitate it; the server never sends a guest as `kind: member`. |
| Offensive names | Not blocked by a word list in the core: lists are language- and culture-specific and wrong in both directions. The server has a `NameFilter` hook an operator can fill with its own list. The owner can *rename a guest* to "Guest 4821" and *remove a guest* (close the session, delete the ticket). A banned guest can re-enter the link as a new guest; the remedy for persistent abuse is turning that link off. |

Member names from tokens pass the same character rules but are never refused: control and format characters are
removed and the length is clipped to 64 grapheme clusters. The embedder is trusted for names but not for rendering
safety.

### Roles and the permission matrix

Roles come from the token (members) or the link (guests). "Guest-E" is a guest on an editor link and "Guest-V" a
guest on a viewer link. Cells: **Y** allowed; **n** denied; **L** allowed but under the stricter guest limits;
**N/A** the case does not exist.

| Operation | Owner | Editor | Viewer | Guest-E | Guest-V | Enforced in |
|---|---|---|---|---|---|---|
| Open the surface, receive objects for the view | Y | Y | Y | Y | Y | room (`Subscribe`) |
| Send own view, cursor, selection | Y | Y | Y | Y | Y | room |
| Send live ink while drawing | Y | Y | n | L | n | room |
| Create, move, restyle, reorder, group or delete own objects | Y | Y | n | L | n | room, per change op |
| Change or delete objects made by someone else | Y | Y | n | L | n | room |
| Edit text; place a library template | Y | Y | n | L | n | room |
| Play, pause or seek a video | Y | Y | n | L | n | room |
| Save a user library (a small surface) | Y | Y | n | n | n | room |
| Upload an image | Y | Y | n | L (8 MiB) | n | upload grant, checked on every upload request and again at blob commit |
| Upload a video | Y | Y | n | n | n | upload endpoint |
| Upload a font | Y | Y | n | n | n | upload endpoint |
| Ask for a blob grant | Y | Y | Y | Y | Y | room: only for a blob in the surface's current state, or in a snapshot or version the role may open |
| Ask for an upload grant (`UploadGrantRequest`) | Y | Y | n | L | n | room; checked again on every upload request against the session's upload floor; ending the session or losing upload rights cancels earlier grants |
| List or open a version (read only) | Y | Y | n | n | n | room |
| Create a named version | Y | Y | n | n | n | room |
| Restore a version | Y | n | n | n | n | room |
| Delete a named version | Y | own only | n | n | n | room |
| Copy content out of a version | Y | Y | n | n | n | room |
| Delete everything created by a guest or link (`Op::DeleteBy`) | Y | n | n | n | n | room |
| Freeze guests | Y | n | n | n | n | room |
| Change surface settings (limits, bounds, home, name) | Y | n | n | n | n | room |
| Create, list, change, turn off share links | Y | n | n | n | n | room and admin API |
| Rename or remove a guest | Y | n | n | n | n | room |
| Manage members | N/A | N/A | N/A | N/A | N/A | the embedder's tokens |
| Delete the surface | Y | n | n | n | n | admin API; room for the owner |
| Export the whole surface | Y | Y | tenant setting, default n | n | n | export endpoint |
| Read the surface's audit trail | Y | n | n | n | n | room |
| Jump to or follow a person | Y | Y | Y | Y | Y | room; target must be `locatable` |
| Be jumped to | Y | Y | Y | Y | Y | the target's `locatable` flag |
| Change own name (member: not at all; guest: rules above) | n | n | n | L | L | room |
| Toggle own `locatable` | Y | Y | Y | Y | Y | room |

Rules that sit under the matrix:

- **Enforcement is in the room, on the server.** The client hides buttons it cannot use, which is convenience and
  not control. `Authz::check` runs on every incoming message and on every HTTP request, with the *current* session
  role, so a downgrade by refresh or by a link change takes effect on the next message.
- **A denied operation is dropped whole.** A change message with one denied op is refused entirely, not
  partially applied, so a half-applied change cannot appear. The sender gets `denied { op, change_id }`; the room
  does not tell other clients.
- **Role of objects.** Per-object locking does not exist (high-level design). An editor may change anything an
  editor can reach. Guest-E may too, which is the "deface until turned off" risk; limits and the owner actions
  above bound the damage.
- **Viewers and the log.** A viewer receives the same objects an editor would for that view. There is no partial
  hiding inside a surface.
- **Export of a whole surface** reads everything, which view-scoped sync was designed to avoid; it runs as a
  server-side job under the requester's role and is audited. Whether viewers may export is a product decision
  (question 4).
- **Jump to a person** returns the target's view only if the target is `locatable`. A refusal looks the same as a
  person who has left.

### The opt-out from jump and follow

The high-level design accepts that the people list and "jump to this person" reveal where someone is working. The
opt-out is cheap and is adopted:

- Each session has `locatable: bool`, default from the token's `loc` claim, otherwise true; the person can flip it
  at any time. It is enforced in the room: when false, the people list shows the person with no jump action, and a
  `JumpTo` request for them is refused as above. Their view is never put in any message to another client.
- **What it does not hide.** Requirement 7 still shows a cursor to anyone whose view contains it, and that
  reveals position to those people. That is the product's point, so the opt-out covers only the *on request*
  reveal.
- **Follow** is a server-pushed stream of the target's view at 15 updates a second, one stream per following
  session (section 04). A request to follow needs `locatable`, is shown to the target ("Ana is following you") and
  ends when either side turns it off. Jumps keep their own limit of 1 per second and 30 per minute, so polling a
  person's position by jumping is no cheaper than following openly.
- Cost: one boolean in the session and one check; the presenter case ("everyone follow me") works because the
  presenter leaves it on. If the owner wants presenters to be unable to hide, that is a surface setting
  (question 5).

### Limits

This is the single home of every numeric limit in the design. Other sections name a limit and point here. A row that
says "set by section N" has its number and reasoning in that section; this table lists it so that the server has one
list to load, and the number is repeated here only when it is a security cap.

Every number is a **default**, set per tenant and overridable per surface by the owner within the tenant's ceiling.
"Guest" numbers apply to both guest roles. Byte sizes assume about 12 bytes per stroke point on the wire (section 03
may adjust, and then these follow it).

In the last column, "Own proposal" marks a number this section chose because no other section gave one; it is a
placeholder to be tested, not a derived figure. Rows that name a section take their number from it.

| Limit | Member | Guest | How it was derived |
|---|---|---|---|
| WebSocket message, hard transport cap | 1 MiB | 256 KiB | Own proposal: Above the largest legal change; larger is dropped before parsing. |
| `MAX_CHANGE_BYTES`: change message (one commit), size | 256 KiB | 64 KiB | Own proposal: 20,000 points x 12 B = 240 KB; 5,000 x 12 B = 60 KB. |
| Unsent bytes allowed on a connection before bulk writes pause | what the connection drains in about 20 ms, clamped to 4 KiB to 64 KiB | same | Section 03: the server estimates the drain rate from how fast writes complete; a cursor waits about 40 ms at most behind bulk data above about 1.6 Mbit/s; below that the 4 KiB floor applies and the server sends fewer objects first by raising `eff_min_px`. 20 ms is the first value the spike tries. |
| Bulk chunk written to the socket | no larger than the unsent-bytes limit, never over 16 KiB | same | Section 03. |
| `MAX_OPS_PER_CHANGE`: operations in one change (objects created or changed) | 1,000 | 100 | Own proposal: A pasted library template or a group move. |
| `MAX_STROKE_POINTS`: points per stroke | 20,000 | 5,000 | Own proposal: A pen at 240 Hz for 83 s; clients simplify, so real strokes are far smaller. |
| `MAX_CUTS_PER_STROKE`: cuts per stroke | 64 | 64 | Section 02: past 64 the eraser replaces the stroke with pieces. |
| `MAX_TEXT_CHARS`: text length, one text object (scalar values) | 50,000 | 5,000 | Own proposal: An essay for members; a paragraph for guests. |
| `MAX_TEXT_UPDATE_BYTES`: one text update | 64 KiB | 16 KiB | Own proposal: Fits inside the change cap; a larger paste becomes several objects (section 02). |
| `MAX_ORDER_KEY_BYTES`: order key length | 64 | 64 | Own proposal: Section 02 rebalances at 24 bytes, so 64 is far above any honest key and bounds a hostile one. |
| `MAX_TREE_DEPTH`: group nesting depth | 16 | 16 | Own proposal: Recursion bound for every walk of the tree. |
| Style runs per text object | 2,000 | 200 | Own proposal: Layout cost grows with runs. |
| Path or polygon vertices, one shape (path nodes) | 20,000 | 5,000 | Own proposal: Same as strokes; section 06 adds its own render skip. |
| Glyphs per text block | 60,000 | 6,000 | Own proposal: one glyph per scalar value at most, so slightly above `MAX_TEXT_CHARS`; section 06 points here. |
| Encoded size of one overview picture | 512 KiB | 512 KiB | Own proposal: a 512 x 512 RGBA tile is 1 MiB raw (section 06); a larger encoding is refused as hostile. |
| Time to render one tile before it is marked failed | 2 s | 2 s | Own proposal: section 06 asks for a time-out and gives no figure; measure in the spike. |
| Far objects sent per view (`max_far_objects`) | 256 | 256 | Own proposal: section 03 says a few hundred is enough for any honest nesting. |
| Unacknowledged budget in `Welcome.window` | 1 MiB and 256 changes | 1 MiB and 64 changes | Section 03's proposal (four full-size member changes in flight), held here; a tuning target for the spike. |
| A child's distance from its group, in levels | 8 | 8 | Section 02. |
| Followers per person (`max_followers_per_person`) | 50 | 50 | Section 04: a 51st follower is refused with a message; reasoning in section 04. |
| Blob ids in one `BlobGrantRequest` | 100 | 100 | One number for the whole design; section 03 names the message. |
| Descendants of one group (`max_group_descendants`) | 20,000 | 20,000 | Reasoning in section 03. |
| Objects per view (`max_view_objects`) | 50,000 | 50,000 | Reasoning in section 03. |
| Bytes per view (`max_view_bytes`) | 64 MiB | 64 MiB | Reasoning in section 03. |
| Objects on a surface | 2,000,000 | shared | Section 03: the whole surface is held in server memory. |
| Objects a guest session may create in total | n/a | 5,000, then read-only | Own proposal: Bounds defacement per link visit. |
| Bytes of changes a session may send in total | n/a | 30 MiB | Own proposal: Same. |
| Position depth, levels | within +-1024 | same | Section 01. |
| Encoded position, bytes (cell magnitude at most 12,288 bits) | about 3.1 KB | same | Section 01. Checked on the raw bytes before any number is built. |
| Image upload, file size | 50 MiB | 8 MiB | Member figure from section 05; guests stricter. |
| Image pixels | 128 Mpx, each side at most 32,768 | 16 Mpx, each side at most 8,192 | Section 05 for members (128 Mpx x 4 B = 512 MiB, bounded by the worker's limits); guests stricter. |
| Video upload | 2 GiB, 4096 x 2304, 4 hours (section 05) | not allowed | Section 05 for members. |
| Font upload | 20 MiB (section 05) | not allowed | Section 05. |
| Placeholder, bytes | 64 | 64 | Set by section 05. |
| Alt text, bytes | 500 | 500 | Set by section 05. |
| Playback writes per second | 5 per user, 10 per object | not allowed | Set by section 05. |
| Playback anchor tolerance | 2 s | n/a | Set by section 05. |
| Decoder memory; decode time | 768 MiB (`max_alloc`), 1.5 GiB process; 60 s wall, 45 s CPU | same | Set by section 05. |
| Image metadata (EXIF, XMP, ICC) | 1 MiB | 1 MiB | Set by section 05. |
| Font glyphs; variation axes | 65,535; 16 | not allowed | Set by section 05. |
| Presigned address lifetime | 60 s | 60 s | Set by section 05. |
| Uploads in flight | 3 per user, one a video (section 05) | 1 | Section 05; guests stricter. |
| Blob storage per surface | 2 GiB | shared | Own proposal: Tenant ceiling decides. Unreferenced uploads are deleted after 24 h. |
| `max_visible_cursors`: cursors drawn per receiver (banners: 50) | 200 | 200 | Reasoning in section 04. |
| `presence_down_budget`: presence bytes per receiver | 32 KB/s | 32 KB/s | Reasoning in section 04 (256 kbit/s). |
| `presence_room_egress`: presence bytes per room | 100 Mbit/s | shared | Reasoning in section 04. |
| `max_live_strokes`: strokes being drawn, per room | 64 | shared | Section 04 assumes 64; with 2,000 sessions that is 128,000 rectangle tests per run. |
| Clipboard payload read by the shell (custom type, HTML item or plain text, bytes scanned) | 4 MiB | 1 MiB | Own proposal: Section 07 enforces it before anything is parsed; a pasted image is also bound by the image rows; the objects inside are bound by the change rows. |
| Presence message size | 1,000 B | 1,000 B | Section 04: moves to datagrams later unchanged. |
| Sockets per surface | 500 | 200 guests inside the 500 | Own proposal: Room memory; section 04's 65,535-session maximum is above it. |
| Sockets per user (`(tenant, sub, surface)`) | 8 | 1 per `GuestId` | Own proposal: Several tabs; not hundreds. |
| Unauthenticated sockets per address | 64 | 64 | Own proposal: A school behind one address can still connect; raise per tenant. |
| Authenticated guest sockets per address | n/a | 128 | Own proposal: Same. |
| Sockets per tenant | 10,000 | included | Own proposal: Fairness between tenants. |
| Display names (grapheme clusters) | 64 | 40 | Own proposal: See rules. |
| Share links per surface | 5 | | Own proposal: See above. |
| Named versions per surface | 100 | | Own proposal: Storage. |
| Deleted objects and automatic snapshots kept | 30 days | | Sections 02 and 03; named versions are kept until deleted. |

**Rates.** Each session has its own limiters; there are also aggregates per user and per address. A message that
does not fit is refused (changes, with `rate_limited { retry_after }`) or dropped (presence). Cost units: 1 per
message plus 1 per 4 KiB.

| Stream | Member, sustained / burst | Guest, sustained / burst | Reason |
|---|---|---|---|
| Presence messages (cursor, selection, live ink points) | 60 /s / 120 | 30 /s / 60 | Section 04: at most 60 a second (30 for guests); clients send about 30 /s. |
| Change commits | 20 /s / 100 | 5 /s / 20 | Normal drawing commits a stroke every 0.2 s or slower |
| View changes | 20 /s / 40 | 20 /s / 40 | Panning; each costs an index query |
| Upload requests | 10 /min | 3 /min | |
| Blob grant requests | 60 /min | 60 /min | Each carries at most 100 blob ids. |
| Upload grant requests | 10 /min | 3 /min | |
| Jumps | 1 /s, 30 /min | same | See opt-out. Follow is not a jump: it is one server-pushed stream of 15 updates a second per session (section 04). |
| Joins (exchange) | n/a | 10 failures /min per address | |
| Bytes in, per session | 512 KiB/s | 128 KiB/s | Above any honest stroke rate |

**Algorithm and scopes.** GCRA, through `governor`: one limiter per connection, one per user key `(tenant, sub,
surface)` at 2x the per-connection figure, and one per address (guests and pre-authentication only) at 8x the
per-connection guest figure and 4 MiB/s. Aggregates stop a client from multiplying its rate by opening sockets.
Address keys come from the socket, or from `X-Forwarded-For` only when the connecting peer is on the operator's list
of trusted proxies (otherwise a client could pick its own address). IPv6 is keyed by /64. Tables of keys expire idle
entries after 10 minutes.

**What happens at the limit.** Presence over the limit is dropped without reply. A change over the limit is refused
with `rate_limited`. Ten refusals in ten seconds close the socket (4429). A second close within 5 minutes bans the
user key or address from that surface for 5 minutes, doubling up to 1 hour. If more than a third of a link's
guests are being limited at once the room sets the surface to *guests frozen* automatically and tells the owners.

**Compression.** `permessage-deflate` is off, because it adds an inflate step that an attacker can use as a
decompression bomb and a length side channel (RFC 8725 2.4 describes the same effect for tokens). If section 03
decides to compress, inflation stops at the message cap and aborts when the ratio exceeds 20:1.

### Abuse handling

| Abuse | Control |
|---|---|
| Flooding with changes or presence | Rates above; sockets closed and banned with backoff; per-tenant socket cap. |
| Deface as an editor guest | 5 links max; guest write budget (5,000 objects, 30 MiB); smaller changes; auto freeze on mass limiting; owner can *freeze guests*, *`Op::DeleteBy` for this guest or link*, turn the link off, or restore a version. |
| Deface as an editor member | Same tools except the guest budget; the embedder is the one who removes the member (new tokens, revocation floor). |
| Hostile embedding app | Trusted only for its own tenant. Tenant quotas: surfaces, sockets, storage, upload rate, new `sub` values per hour. The operator, not the tenant, sets the quotas and can suspend the tenant. It cannot reach another tenant (below). Its content is still escaped and limited like anyone's. |
| Memory exhaustion by deep positions | Encoded-size and level checks run on the raw bytes before any big-number allocation. |
| Memory exhaustion by huge objects | Size and count limits above are checked at the framing layer, then the parser stops at the cap while parsing (no "parse then check"). The room has a memory budget; at 90% it refuses object creation and keeps serving reads. |
| Slow-loris (slowly sent headers, frames or reads) | Header deadline 10 s; authentication frame 5 s; a partly received message must finish in 10 s; ping every 20 s, closing if no pong in 20 s; the outbound queue per socket is capped at 4 MiB, presence dropped first, then the socket closed; idle 5 minutes without application traffic closes the socket. |
| A hand-made change from a viewer, or a text update from a guest | The server decodes every object change itself, checks each field against the role, and accepts or rejects the whole change; text updates are checked separately (see assumptions). |

### Tenancy

- Rooms are keyed `(TenantId, SurfaceId)`. Persistent storage, the change log, snapshots, versions, link tables,
  blob keys, audit entries and rate-limit tables are all prefixed or keyed by tenant, and every function that
  touches them takes a `TenantScope`. There is no function to look up a surface by its id alone.
- Keys are per tenant (connect keys, admin keys). The path's tenant selects the key set and the token's `iss` must
  agree; so a token for tenant A does not open tenant B's surface even if both used the same surface id.
- Blob deduplication is keyed `(tenant, hash)`. Two tenants uploading the same file store it twice. An upload's
  answer never depends on whether another tenant has the file (the server always receives the whole file and
  computes the hash itself).
- Within a tenant, a blob is readable only through a **blob grant** `(tenant, surface, hash)`, created when the
  bytes are uploaded through that surface, or when the server copies content from a surface the copier can read to
  another it can write. A change message that names a hash with no grant for that surface is refused. This closes
  the gap in correction 3.
- Resource isolation: per-tenant quotas, set only through the operator interface and never by the tenant (sockets,
  rooms, storage bytes, upload bytes per day, token-verification failures) and a per-tenant CPU share through a
  bounded work queue per tenant. An operator who needs stronger separation runs one server process per tenant; the
  design does not stop that, and the default of a single tenant `default` makes the common case a one-line
  configuration.
- A tenant can be suspended or deleted by the operator. Deleting a tenant deletes everything under its scope
  (see Retention).

### Blob access control

A **blob grant** is one signed address prefix that lets a session read one blob and all its derived files (sizes of
an image, video tiles and posters, font variants). The room, not the blob store, decides whether to issue it, and
issues it only if the session passes `Authz::check` for the surface and the surface references the blob (the
tenancy rule above). The request names blobs; the answer never says whether a hash exists elsewhere.

```
https://<blob origin>/b/<tenant>/<blob hash>/<file name>?s=<surface>&sid=<session id>&ep=<lease epoch>&exp=<unix s>&k=<key id>&sig=<mac>
sig = HMAC-SHA256(blob_key[k], "everink-blob-v1\0" || tenant || "\0" || surface || "\0" || blob hash || "\0"
                              || session id || "\0" || epoch || "\0" || exp)
```

- **What is signed:** tenant, surface, blob hash, session id, the room's lease epoch and expiry. The file name (a
  derived size or tile) is not signed, so one grant covers every derived file of the blob and a client does not ask
  again for each zoom level. The session id ties the grant to the session that asked for it; a revoked session fails
  below. The **lease epoch** is section 03's: the increasing number a room takes with its lease when it is loaded
  (see "Placement" in section 03), written into the room file's `meta` and held durably in the coordination store.
  It changes whenever the room is loaded again after a crash, a restart or an idle unload, so it names one
  incarnation of the room.
- **Lifetimes:** **10 minutes** for images, tiles and fonts. **2 hours** for video, because a browser keeps sending
  range requests to one address and cannot be handed a new one in the middle of playback. Clients ask again before
  expiry. The store checks the MAC in constant time, the expiry (5 s leeway) and that the key id is known. Keys
  rotate every 90 days, with an overlap of 24 hours (the longest upload grant).
- **The epoch check.** The blob endpoint and the upload endpoint refuse any grant whose epoch is not the room's
  current one. They read the current epoch from the durable lease (the coordination store; on a single node the same
  number in the room file's `meta`, read by the same process), with a cache of at most 1 s, never from the memory of
  a room that may have restarted. A client whose grant is refused for an old epoch reconnects, which a restart
  forces anyway, and asks for new grants. So every grant dies when its room incarnation ends, whatever was in
  memory.
- **Revoked sessions:** the list lives in `everink-server`, which serves the media endpoint. The room adds the id of
  every session that ends by revocation (link turned off, guest removed, revocation floor), before it answers the
  owner who turned the link off. The endpoint checks every blob read request and every upload request (`POST`,
  `PATCH`, `HEAD`) against it, so turning a link off cuts blob access at once, not after the grant would have
  expired. The list only has to live as long as one incarnation of the room: after a restart the epoch has changed
  and every earlier grant is refused by the epoch check, so an empty list cannot let a removed guest's grant
  through. A session that ends normally needs no entry; its grants die at the epoch change or expire. The list is
  small: it holds only forced endings of the current incarnation, 16 bytes each. If the endpoint cannot reach the
  list or the lease, it refuses video requests it cannot check (fail closed) and keeps serving cached image grants
  only until their 10 minutes end.
- **The user is not in the read MAC,** so a read grant is a capability: whoever holds the address can read until it
  expires or its session is revoked. Binding it to a cookie or header is impossible because a media element cannot
  send headers.
- **Upload grant.** The one credential for uploads. The room issues it on an `UploadGrantRequest` after
  `Authz::check` for the upload operation and the limits table (size, kind, uploads in flight). It is bound to the
  principal, the surface, the blob and its declared size, and is valid while the session lives and its role keeps
  upload rights, and for at most 24 hours. Section 05 and section 03 point here.

  ```rust
  struct UploadGrant {
      tenant: TenantId, surface: SurfaceId, principal: PrincipalKey, session: SessionId,
      blob: BlobHash, size: u64, kind: UploadKind,
      epoch: u64,            // the room's lease epoch; the endpoint refuses any other
      issue_seq: u64,        // from the room's counter within this epoch; compared with the session's upload floor
      expires: Timestamp,    // expires <= issued + 24 h
      mac: [u8; 32],   // HMAC-SHA-256, context "everink-upload-v1", over every field above
  }
  ```

  The upload endpoint refuses a request whose bytes exceed `size`, whose blob differs, whose `epoch` is not the
  room's current lease epoch, whose session is on the revoked list, or whose `issue_seq` is not above the session's
  **upload floor**, on every `POST`, `PATCH` and `HEAD`. So an upload that was in progress when a link was turned
  off stops at its next request.
- **The upload floor.** Because a grant is a signed value that stays good until it expires, the room keeps for each
  session an *upload floor*: a number, in the list `everink-server` holds beside the revoked sessions (the
  cancelled-uploads list). The floors and the `issue_seq` counter live in memory for one incarnation of the room
  only: a new incarnation has a new lease epoch, so grants from the old one are refused before `issue_seq` is looked
  at, and the counter may restart. A grant is good only if its `issue_seq` is above its session's floor. The room
  raises the floor to its current counter when the session ends for any reason (a normal close, a lost connection, a
  token that expires without refresh), when a refresh or a change of link role removes upload rights (editor to
  viewer, or an editor link changed to a viewer link), and when guests are frozen. So an unexpired grant cannot
  upload after any of these.
- **Raising rights again.** The floor is never lowered or deleted while old grants could still be alive. When a
  freeze is lifted, or a role is raised again, the session simply gets new grants whose `issue_seq` is above the
  floor, so they work, while grants issued before the cancellation stay dead. The entry is dropped when the session
  ends and its grants can no longer be used, and at the latest with the room incarnation. A new connection is a new
  session with its own floor.
- **Freezing guests does not revoke them.** A frozen guest becomes a viewer at once, keeps read access to blobs
  (its sessions are not on the revoked list), and has its upload floor raised, as above.
- **What a read grant is checked against.** The room issues a read grant only for a blob referenced by the
  surface's current state, or by a snapshot or named version that the requester's role may open. The reference index
  that never forgets is for garbage collection only, never for access. A viewer therefore cannot read a blob that is
  referenced only by a deleted object.
- **Originals.** Derived files are what clients display. The uploader's original is download-only: it lives under a
  different path (`/o/`) with a different MAC context string (`everink-blob-orig-v1`), is sent as an attachment
  (headers below), and the room issues such a grant only to members. Guests never receive one.
- **Cache headers match:** `Cache-Control: private, max-age=<remaining life of the grant>`.
- The signing key is a 256-bit secret held by the server processes (and by edge caches, if any); it is not a
  tenant's key.

**Leakage.** The signature is in a query string. It can reach the blob origin's access logs, a proxy log or a
`Referer`. Controls: the blob origin logs the path without the query; every blob response carries
`Referrer-Policy: no-referrer`; the app sets `Referrer-Policy: no-referrer` too; a blob response is never a page
that links elsewhere; the expiry limits the damage of anything that does leak.

**Response headers the blob store sends on every file.** This table is the only list of them; section 05 refers to
it.

**Presigned mode.** If the store answers with a redirect to a presigned address on object storage, the same headers
must be set there: response-header overrides on the presigned request, object metadata written at upload, and a
bucket CORS policy for the tenant's origins. A store that cannot do all three runs in proxy mode (the server streams
the bytes). The header test runs in both modes.

| Header | Value | Reason |
|---|---|---|
| `Content-Type` | The type the server decided after validating the bytes (`image/webp`, `image/png`, `video/mp4`, `font/ttf`, `font/otf`, ...), never the uploader's | No sniffing by trust in the client |
| `X-Content-Type-Options` | `nosniff` | The browser must not guess a type |
| `Content-Disposition` | `attachment; filename="blob"` | If anyone navigates to the address, it downloads and does not render (does not block use by `fetch` or a media element) |
| `Content-Security-Policy` | `default-src 'none'; sandbox` | If something does render, it has no scripts and a null origin |
| `Cross-Origin-Resource-Policy` | `cross-origin` | The app, if cross-origin isolated, must be allowed to load it |
| `Access-Control-Allow-Origin` | the requesting origin if the tenant allows it, with `Vary: Origin` | The wasm app uses `fetch` |
| `Referrer-Policy` | `no-referrer` | See leakage |
| `Cache-Control` | `private, max-age=<remaining life of the grant>` | No shared cache outlives the grant |
| `Accept-Ranges` | `bytes` | Video |
| `Strict-Transport-Security` | `max-age=31536000` | |
| `Set-Cookie` | never | The blob origin has no state |

**A separate origin.** Blobs are served from a different *site* than the app and the API (a different registrable
domain, not just a subdomain, for example `example-usercontent.net` for `example.com`), so no cookie, storage or
service worker of the app is reachable even if a file were rendered. Google also separates user content onto its own
domain (unverified here; widely known). The blob origin has no cookies, no login and no path other than `/b/` and
`/o/`.

### Media and font threats

The wording is a requirement for section 05; this section sets the line.

| Threat | Control |
|---|---|
| Decompression bomb (a small PNG that inflates to gigabytes) | Section 05's rule: read the header, compute the size, compare with the limits table, only then decode. The media worker is a separate OS process with memory and CPU limits, no network and no filesystem beyond its pipes (limits and times are section 05's). Fonts: TTF and OTF only, probe and table-size caps; WOFF2 is refused in the first version. Animated images are refused (one frame). |
| Polyglot (a file valid as an image and as something else, such as a script or archive) | The worker decodes images and writes derived files in its own formats; clients display only derived files. Metadata (EXIF, which can hold GPS position) is dropped from derived files. The original is kept as a download-only attachment for members, served from the `/o/` path with `attachment`, `nosniff`, `sandbox`. Video is never decoded or re-encoded by the server (section 05): it is probed by header only, then served as `video/mp4` or `video/webm` with `nosniff` from the separate origin. |
| SVG and embedded scripts | SVG is not an accepted upload type, as image or as font or anywhere. HTML, PDF, ZIP and Office files are not accepted either. Vector content comes only through Everink's own object types. |
| Content sniffing | `nosniff`, a server-chosen `Content-Type`, `Content-Disposition: attachment`. |
| Parser memory-safety bugs | Parsers are memory-safe Rust or run in the sandbox; the font path reads tables and draws glyph outlines with Everink's own code and never hands bytes to the operating system's font stack or to the browser's. No hinting program is executed. |
| Platform video decoders | The server never decodes video; the browser's or the OS decoder does, which is outside our control and outside memory-safe code. Controls: container and codec allow-list (section 05), member-only upload, per-tenant switch to turn video off, header probe with size and duration caps, `sandbox` on the blob origin. Residual risk accepted and named. |
| Filename tricks, path traversal | Files are named by hash; the uploader's filename is not kept. |
| Pasted content | **Paste rule:** foreign HTML on the clipboard is never parsed, inserted or rendered; only its plain-text sibling and images are used. Everink's own objects travel in a custom clipboard type and in a `data-everink` attribute inside the HTML item, found by a bounded byte scan (the clipboard-payload limit), never by an HTML parser. Text is plain runs. Section 07 implements it. |
| Name and text reaching the DOM | The canvas draws all names and text. The accessibility mirror (section 07) must set text with `textContent` or an equivalent, never as markup. |

### Web client hardening

The embedding app serves the page and so sets its headers; Everink ships a recommended set and a test that runs the
web shell under it.

```
Content-Security-Policy:
  default-src 'none';
  script-src 'self' 'wasm-unsafe-eval';
  connect-src 'self' wss://<server> https://<server> https://<blob origin>;
  img-src 'self' blob: data:;  media-src https://<blob origin> blob:;  font-src 'self';
  style-src 'self';  worker-src 'self';
  object-src 'none';  base-uri 'none';  form-action 'none';  frame-ancestors <embedder origins>;
  require-trusted-types-for 'script'
Referrer-Policy: no-referrer
X-Content-Type-Options: nosniff
```

- Images are fetched and decoded by the wasm code, not by `<img>`, so the blob origin is in `connect-src`;
  `media-src` carries it for video elements; `img-src` stays `'self' blob: data:`. Section 07 uses this policy.
- `'wasm-unsafe-eval'` lets WebAssembly compile and does not allow `eval` or `Function` (MDN). The plain
  `'unsafe-eval'` must never be needed: the web shell and its generated glue must not use string-to-code APIs. The
  CI test loads the shell under this policy and fails on any violation report. (Trusted Types compatibility with
  the generated glue is unverified and may force dropping that last directive.)
- **No dynamic code,** no remote scripts, no inline scripts, no third-party scripts on pages that can hold a link
  secret in the fragment. Scripts are self-hosted, so Subresource Integrity is not the main control.
- **Cross-origin isolation** (`Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy:
  require-corp`) is needed only if the web shell uses threads through `SharedArrayBuffer` (web.dev). It has costs:
  every cross-origin resource needs a `Cross-Origin-Resource-Policy` header or CORS (the blob store sends them), and
  an embedder that puts Everink in an iframe must itself be isolated, which many cannot do. **Section 07 runs the
  core single-threaded, with no wasm threads,** so neither header is needed and embedding works anywhere. A tenant
  that later adds threads can opt in; the blob headers above already allow it.
- Nothing here depends on the renderer. The page may run on WebGPU, WebGL2 or the CPU renderer (section 06);
  all three run under the same policy, and the CI test runs the shell on each.
- `frame-ancestors` lists the embedder's origins, which stops other sites from framing the surface (clickjacking)
  and from capturing a link page.
- Local storage: IndexedDB or OPFS holds the cache and offline changes keyed by `(tenant, surface, principal)`.
  "Sign out" and "leave" delete that keyed data. Guest tickets are in `sessionStorage` only.

### Privacy, logging and retention

**What presence reveals.** Other clients receive: the name, color, `kind` (member or guest), the cursor if it is
inside the receiver's view, the receiver's own per-view selection data as in section 04, and on request a target's
view if `locatable`. They never receive the user id (`sub`), the address, the link id, or a view that was not
requested. The `GuestId` is shown to the owner only.

**Logging.**

| Logged (operational log) | Never logged |
|---|---|
| Time, request id, tenant, surface, session id, role, principal kind, operation name, outcome code, sizes, timings | Tokens, tickets, upload grants, share-link secrets, signed addresses, query strings of blob requests, message contents, names of guests, object contents |
| Remote address truncated to /24 (IPv4) or /48 (IPv6) | Full remote address (kept in memory only for rate limiting) |

The secret types print as `[redacted]` in `Debug` and `Display`, so a stray `{:?}` cannot leak them. A test
greps log output of the integration tests for the test secrets.

**Retention (defaults, operator may shorten).**

| Data | Kept |
|---|---|
| Operational log | 14 days |
| Audit trail | 400 days, then deleted; entries keep the truncated address for 30 days |
| Automatic snapshots | 30 days (high-level design) |
| Named versions | Until deleted |
| Guest tickets | Until expiry, at most 24 hours |
| Unreferenced uploads | 24 hours |
| Revoked or expired links | Hash and metadata 90 days, for the audit trail, then deleted |

**Deletion of a surface.** The owner (or the tenant through the admin interface) deletes a surface. The room closes,
then a job removes: document log, snapshots, named versions, link records, tickets, the spatial index, overview
pictures, and the surface's blob grants; a blob with no remaining grant in the tenant is deleted from the store.
Blob grants issued earlier die within 2 hours at most, at once if the sessions were revoked, and at the next load of
the room. Backups of the store age out under the operator's backup policy (the operator must say how long; 35 days
is a common figure, unverified as a norm). The audit trail keeps a tombstone (surface id, who deleted, when) for 400
days. The job is idempotent and recorded, and a test restores nothing from it. Whether deletion should wait a grace
period for undo is a product question (question 6).

**Export.** The owner can export the surface (format and mechanism in section 03). It runs under the owner's
role and is audited. A person's right to their own personal data under privacy laws is the embedder's to meet; the
server stores `sub` and the typed name only, and the embedder can delete by `sub` through the admin interface
(`erase_user`, which replaces `sub` in audit and creator fields by a stable pseudonym). Legal sufficiency is not
assessed here (unverified; needs counsel).

### Transport security and encryption at rest

- All client traffic is `wss` and `https`. TLS 1.2 or later, 1.3 preferred, modern cipher suites only. The server
  terminates TLS itself with `rustls` (0.23.x stable) or sits behind the operator's proxy; it refuses to listen on a
  non-loopback address without TLS unless started with `--behind-tls-proxy`, which also requires a trusted-proxy
  list. Browsers see `Strict-Transport-Security: max-age=31536000; includeSubDomains` on the API and blob origins.
- Plain `ws` and `http` are accepted for `localhost` development only.
- **At rest:** the server stores through a trait; encryption belongs to the storage layer: volume encryption for a
  directory store, server-side encryption for an S3-style store, keys with the operator. The server's own secrets
  (blob key, admin keys' public halves, TLS key) come from the environment or a secret manager and never from the
  repository or a command line. Application-level encryption per tenant adds little against an operator who reads
  everything (decision 5) and costs key-management complexity; it would protect a stolen backup. Not in version 1
  (question 7).

### The audit trail

An append-only table per tenant, one row per event, each row carrying the hash of the previous row so deletion or
change in the middle is detectable by a verifier. The operator may export the chain head to an outside store.

```rust
struct AuditEvent {
    id: Ulid,
    time: Timestamp,
    tenant: TenantId,
    surface: Option<SurfaceId>,
    actor: Actor,            // Member(sub) | Guest(guest_id, link) | Admin(key id) | System
    session: Option<SessionId>,
    action: AuditAction,
    target: Option<String>,  // link id, version id, guest id, key id; never a secret
    outcome: Outcome,        // Allowed | Denied
    prev_hash: [u8; 32],
}
```

Recorded: session open and close (with role, `kid` or link id); a role seen to change on refresh (old, new); denied
operations (aggregated per session per minute, so a flood does not flood the log); link created, role changed,
turned off, expired; guest renamed or removed; guests frozen or unfrozen; `Op::DeleteBy`; version created,
restored, deleted; surface settings changed; surface deleted; export; key added or removed; revocation floor set;
tenant suspended; `erase_user`. Who may read: owners see their surface's rows; the tenant admin sees the tenant's.
Row content never includes message bodies.

### Supply chain

Controls for the repository and its releases; the commands go into `AGENTS.md` when code exists.

- **`cargo-deny`** (0.20.2, MIT OR Apache-2.0) in CI on every pull request and nightly: advisories (from the RustSec
  database), licenses (allow-list: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Unicode-3.0, Zlib, CC0-1.0,
  MPL-2.0 for the container probes of section 05 (file-level copyleft; used unmodified as a dependency, so it does
  not reach our own files), and `OpenSSL` only if `aws-lc-sys` needs it, unverified; no other copyleft), bans
  (duplicate versions of crypto crates, known-bad crates), sources (crates.io only; no git dependencies without a
  recorded reason).
- **`cargo-audit`** (0.23.0, Apache-2.0 OR MIT) nightly against `Cargo.lock`, so a new advisory on an already
  merged lock file opens an issue. It overlaps `cargo-deny` and is kept for the schedule.
- **`cargo-vet`** (0.10.3, Apache-2.0/MIT): every dependency that handles untrusted input or secrets (token, crypto,
  TLS, decoders, parsers, async runtime, HTTP, WebSocket) needs an audit entry, ours or imported from a
  publisher the project chooses to trust. Other crates are `safe-to-run` exempt at the start, and the exemption list
  shrinks over time. Which publishers' audits to import is not decided (unverified which publish them now).
- **Locked builds:** `--locked` everywhere, `Cargo.lock` committed, `rust-toolchain.toml` pins the compiler, the
  `wasm-bindgen` CLI version equals the crate version, and `wasm-opt` is pinned. New or updated dependencies come
  with a diff review (licenses, maintainers, transitive count) as the security playbook requires.
- **GitHub Actions:** every action pinned to a full commit SHA; workflow `permissions:` default to `contents: read`;
  no `pull_request_target` running pull-request code; no secrets in forked-PR workflows; cache keys that a pull
  request cannot poison for the main branch; release jobs run on protected tags only.
- **Reproducible web build:** the release `.wasm` is built in a pinned container with `--locked`,
  `--remap-path-prefix`, a fixed `SOURCE_DATE_EPOCH`, and no timestamps; CI builds it twice and compares SHA-256.
  Whether the whole `wasm-bindgen` plus `wasm-opt` chain is bit-for-bit reproducible is **unverified** and is a
  spike task; if it is not, the fallback is a published hash plus build provenance.
- **Provenance:** release artifacts carry a signed build attestation (GitHub's artifact attestations or Sigstore;
  details unverified) and a SHA-256 list. Whether and where to publish crates and packages is an owner decision.
- **Unsafe code:** `#![forbid(unsafe_code)]` in `everink-server`'s auth, limits and audit modules; other crates
  document each `unsafe` block. Parsers (token, change, name, position, image header, font table) have fuzz targets
  that run for a short time in CI and longer nightly.

## Threat model

Frame: STRIDE (Spoofing, Tampering, Repudiation, Information disclosure, Denial of service, Elevation of privilege).

**Assets.** A1 surface content. A2 blobs. A3 tenant connect keys and admin keys. A4 share-link secrets and guest
tickets. A5 the blob signing key. A6 the audit trail. A7 availability of the server. A8 users' privacy (identity,
location in the surface). A9 the build and release pipeline and the client code users run.

**Actors.** Anonymous network attacker; hostile web page (CSWSH); guest on a viewer link; guest on an editor link;
viewer member; editor member; owner member; hostile or compromised embedding app; another tenant; hostile file
creator; supply-chain attacker. The operator is trusted (accepted residual risk).

"ASVS" gives chapters of version 5.0.0, matched by chapter title.

| # | Threat (STRIDE) | Actor and entry point | Control | Tested by | Residual risk | ASVS |
|---|---|---|---|---|---|---|
| T1 | S: forge a token | Network; `auth` frame | EdDSA only, `typ`, `kid` from tenant set, signature, `aud`, `iss`, `surface`, `exp` | N1 to N8 | Embedder key theft (T3) | V9 |
| T2 | S: algorithm confusion, `none` | Network; token header | Verifier fixed to one algorithm; unknown headers rejected | N1, N2 | None known | V9 |
| T3 | S/E: stolen tenant signing key | Compromised embedder | Short tokens; rotation; revocation floor; key removal; tenant scope and operator-set quotas limit the harm to that tenant; a connect key cannot call any admin endpoint | N9, N10, N11 | Up to 10 min of forged access, plus anything the thief did | V9, V13 |
| T4 | S: token replay | Network; stolen token | TLS; 10-minute cap; optional single use | N12 | Replay within the life if single use is off | V9, V7 |
| T5 | S/E: token for tenant A used on tenant B or another surface | Any member | `iss` equals path tenant; `surface` claim equals path; keys per tenant | N5, N6, N13 | None known | V8, V9 |
| T6 | S: cross-site WebSocket hijacking | Hostile web page | No ambient credentials; first-message auth; `Origin` allow-list | N14, N15 | Non-browser clients can lie about `Origin`, which grants them nothing | V3, V4 |
| T7 | I: token or secret in logs, referrers, URLs | Log reader; third-party site | Token never in a URL; secret in fragment, replaced at once; redaction types; no-referrer | N16, N17 | Fragment stays in history of a browser that saved it before replacement | V14, V16 |
| T8 | S/E: guess or steal a share link | Network; link holder | 256-bit secret; hash stored; rate-limited exchange; per-link revocation; expiry | N18 to N22 | A leaked link works until turned off or expired | V6, V7 |
| T9 | E: guest acts as owner | Guest | Link role cannot be Owner; owner-only operations denied by role | N23 to N27 | None known | V8 |
| T10 | E: viewer writes | Viewer or Guest-V | The server decodes each object change itself and checks every field against the role; whole change refused | N28 to N34 | None known beyond bugs in the decoder | V8, V2 |
| T11 | E: hand-made or raw text update bypassing checks | Viewer or guest | Object changes are the server's own typed format, never a library blob; text updates are accepted only for text objects the role may edit, applied to a checked copy | N35 | Text-library limits on inspecting an update (open, question 1) | V8, V2 |
| T12 | S: guest imitates a member | Guest | Skeleton comparison; Guest badge drawn from `kind` | N36 to N38 | Names that look alike in ways the tables miss | V2 |
| T13 | T: editor (guest) defaces | Guest-E, editor member | Budgets; freeze; `Op::DeleteBy`; link off; versions | N39 to N42 | Visible damage until an owner acts (accepted, high-level design) | V2 |
| T14 | R: deny having restored a version, changed a link | Owner or editor | Audit trail with hash chain, actor, session | N43, N44 | The operator can rewrite the table and recompute the chain unless it is anchored outside | V16 |
| T15 | D: flooding changes, presence, uploads | Any client | GCRA at three scopes; caps; bans; auto-freeze; upload grants bound to the session and cancelled by an upload floor when the session ends or loses upload rights, and by the lease epoch when the room restarts | N40b, N45 to N50 | A large distributed flood from many addresses; needs the operator's network protection | V2, V13 |
| T16 | D: slow-loris, half-open sockets | Network | Deadlines on headers, auth frame, messages; ping; queue caps | N51 to N54 | Network-layer attacks below TLS | V13 |
| T17 | D: memory by deep positions or huge objects | Editor or guest | Encoded-size and level checks on raw bytes; caps at framing | N55 to N60 | A tenant raising caps; operator error | V2 |
| T18 | D: decompression bomb | Uploader | Header check against the limits table; media worker as a separate process with memory and CPU limits | N61 to N64 | A decoder bug in the worker | V5 |
| T19 | E/T: polyglot, SVG or HTML upload; content sniffing | Uploader | Type allow-list; clients display derived files only; originals download-only for members; `nosniff`; attachment; separate origin; `sandbox` CSP | N65 to N70 | Video container tricks and platform decoder bugs (accepted) | V5, V3 |
| T20 | E: malicious font | Editor | Memory-safe parser; table allow-list; size cap; Everink-only rendering | N71, N72 | A parser bug; fuzzing reduces it | V5 |
| T21 | I: read a blob without access | Any user; tenant member | One grant per blob, issued by the room after `Authz` and a surface reference; HMAC over tenant, surface, blob hash, session id, lease epoch, expiry; grants refused when the epoch is not the room's current one (so a restart cannot revive a revoked session's grant); revoked-session list on every request; per-tenant dedup | N73 to N78 | A grant works up to 10 minutes (images) or 2 hours (video) after normal access ends, unless the session was revoked | V8, V4 |
| T21a | I: access outlives a revoked session through a presigned storage address | Removed guest or member | Where the store hands out a presigned address to object storage, it is valid for 60 seconds at most and is issued only after the revoked-session check | N80b | Up to 60 seconds of access after revocation (accepted; also stated in section 05) | V8, V4 |
| T22 | I: hash-existence oracle across surfaces or tenants | Member | Server always receives the bytes; grants per surface; dedup per tenant | N79, N80 | Accepted: within one tenant, an upload of a file another surface already holds completes without the processing steps, so the uploader can tell it already existed. They already hold the bytes, so they learn only that it exists in the tenant | V8, V14 |
| T23 | I: cross-tenant data access | Another tenant | `TenantScope` in every call; keys per tenant; room key includes tenant | N81 to N85 | A bug in a new storage path; the type system and tests narrow it | V8, V15 |
| T24 | I: learn where a person is working | Other members | Views never sent except on request; `locatable`; jump rate limit; follow notice | N86 to N89 | A cursor in view reveals position by design | V14 |
| T25 | I: local copy left on shared machine | Next user of the machine | Cleared on sign out and leave; guest ticket in `sessionStorage` | N90 | A user who closes the browser without leaving: cache stays on disk | V14 |
| T26 | T/E: script injection through names, text, files | Any user | Plain runs; canvas drawing; mirror uses `textContent`; CSP | N91 to N94 | A bug in the mirror (07) | V1, V3 |
| T27 | E: supply-chain compromise of a dependency or action | Supply-chain attacker | `cargo-deny`, `cargo-audit`, `cargo-vet`, pinned actions, locked builds, reproducible wasm | CI jobs S1 to S5 | A malicious update that passes review; zero-day in an audited crate | V15, V13 |
| T28 | I: operator reads everything | Operator | None (decision 5) | none | Accepted | V14 |
| T29 | E: hostile embedder reaches beyond its tenant | Embedder | Tenant scope; operator and tenant admin separate, a tenant cannot change its own quotas; separate `typ` and key lists | N81 to N85, N95, N97 | An embedder can read and delete everything in its own tenant, by design | V8 |
| T30 | D: noisy tenant starves others | Embedder or its users | Per-tenant quotas set by the operator only, and work queues | N96, N97 | One process shares CPU; strong isolation needs separate processes | V13 |

## Failure modes

| Failure | What happens |
|---|---|
| The embedder cannot issue tokens (outage) | Connected sessions continue until their token expires (at most 10 minutes), then close with 4401. New joins fail. Guests are unaffected. |
| Clock skew between embedder and server | 30 s leeway; beyond it tokens look expired or from the future, and the log says which. The server exposes its time on the admin interface. |
| A tenant key is lost | The tenant adds a new key through the operator; old tokens fail at once. |
| JWKS fetch fails | Cached keys are used for up to 24 hours; then connections for that tenant fail closed. |
| A room crashes or restarts | All sessions end. Members re-present the token; guests re-present the ticket (stored durably). The link state survives. The room is loaded with a new lease epoch, so every blob read grant and upload grant of the old incarnation is refused, including those of sessions revoked before the restart; the in-memory revoked-session list and upload floors start empty and need nothing from the past. Clients ask for new grants. |
| The media worker dies or exceeds its limit | The upload is refused with a generic error, the worker process is killed and restarted, and the event is audited. |
| The blob signing key is lost or leaked | A new key id is added and the old one removed at once; all outstanding grants die; clients ask again (an interruption of seconds). |
| A link is turned off while guests are drawing | Their sockets close with 4403; their queued changes are discarded by the server when the client reconnects without a ticket; the client shows "link turned off". |
| A downgrade (editor to viewer) arrives with in-flight changes | Changes sent before the room applies the refresh may be accepted; changes after are denied. The order is that of the socket. |
| Audit store unavailable | Denied and security-relevant operations (link, version, role) fail closed; plain drawing continues, with events queued up to 10,000, after which the room goes read-only. |

## How it is tested

Test names are labels (N for a negative test, S for a CI job); each negative test proves a path stays denied and
also checks the audit entry and, where relevant, the absence of a side effect.

| Rule | Test |
|---|---|
| Token verification | N1 `alg: none`. N2 `alg: HS256` with the public key as the secret. N3 expired. N4 `exp - iat` over 600 s. N5 wrong `iss`. N6 wrong `surface`. N7 wrong `aud`. N8 wrong `typ`. And: unknown `kid`; `jku`, `jwk`, `x5u`, `crit` present; duplicate claim; claims over 2 KiB; invalid role value; oversized name. All yield the same close code and the room stays empty. |
| Keys and revocation | N9 removed key refused and its sessions closed. N10 floor revokes. N11 suspended tenant. N12 single use refuses the second use. Property test: for random mutations of a valid token, acceptance implies the signature covers the mutated bytes. |
| Cross-tenant | N13 token A, path B. N81 to N85: for each storage, blob, room, audit and rate-limit function, a test with two tenants using the same surface id and the same hash shows no cross-read or write; a compile-time test that no public function takes a surface id without a `TenantScope`. N95: connect-key signature on an admin endpoint. N97: a tenant admin token cannot set or raise a quota, change a ceiling or touch another tenant, and operator, tenant admin and connect tokens are not interchangeable. N97a an admin token with a reused `jti` is refused, also after a restart. N96: one tenant at its quota does not slow another's p99 beyond a set figure. |
| Origin and handshake | N14 hostile `Origin` refused. N15 a request with cookies and no token gets nothing. Pre-authentication: no data before `auth_ok`; 5-second timeout; frame over 4 KiB; second frame; 65th pre-auth socket from one address. |
| Logging | N16 integration tests run with known secrets and grep every log and error body. N17 `Debug` of every secret type prints `[redacted]`. |
| Share links | N18 wrong secret, expired, off and unknown give identical answers (byte for byte, within a timing tolerance). N19 secret from another surface. N20 rate limit on failures. N21 turning off closes sessions in under 1 s and deletes tickets. N22 expired ticket (24 h) and idle ticket (2 h). Storage test: the table never holds a secret, and a row shows only the hash. |
| Role matrix | A generated test over the table: for every `Operation` and every principal kind (owner, editor, viewer, Guest-E, Guest-V) the expectation in the matrix is asserted, plus the change is not applied, and other clients see nothing. N23 to N27: guest tries each owner-only operation. N28 to N34: viewer and Guest-V try each change type, live ink, upload, version operations. The matrix in this document and the test table are generated from one source file and checked for equality in CI. |
| Server-checked changes | N35 a viewer sends a well-formed object change, and a well-formed text update; both refused. A change with one denied op among permitted ones applies none. |
| Guest names | N36 bidi override, zero-width, control, combining flood. N37 confusable of a present member (Cyrillic "a"). N38 over-length. A fuzz target on the validator. |
| Defacement tools | N39 guest write budget ends writes at 5,000 objects. N40 freeze demotes guest sessions on the next message. N40a a frozen guest can still read blobs, and its upload grants are refused afterwards. N40b an upload with an unexpired grant is refused after a downgrade from editor to viewer, after an editor link is changed to a viewer link, and after a normal session end; an upload with a grant issued after an unfreeze (or a role raised again) succeeds, while the grant issued before it still fails. N41 `Op::DeleteBy` removes exactly that creator's objects and a restore brings them back; a non-owner sending it is refused. N42 auto-freeze on mass limiting. |
| Audit | N43 each audited action produces exactly one row with the right actor. N44 breaking the chain is detected by the verifier. |
| Limits and rates | N44a with a deliberately low drain-rate estimate, a bulk load still completes and the estimate recovers within a few seconds (it does not starve the load), while cursor updates stay within the 40 ms wait. N44b a 51st follower of one person is refused. N45 change flood is refused with `rate_limited`. N46 presence flood is dropped. N47 many sockets from one user share one budget. N48 ban with backoff. N49 trusted-proxy header ignored from an untrusted peer. N50 IPv6 /64 grouping. N51 to N54 slow headers, slow frame, no pong, slow reader. N55 position with level over the cap (+-1024). N56 position with encoded size over the cap of about 3.1 KB (checked before any arithmetic: a test with a counting allocator). N57 change over size. N58 stroke over 20,000 points. N59 text over length. N60 object count. |
| Media | N61 PNG with a large declared size. N62 animated GIF refused. N63 a WOFF2 upload is refused (not accepted in the first version). N64 media worker killed at its limits. N65 polyglot (image plus script or archive): only derived files are displayed. N66 SVG refused. N67 HTML and PDF refused. N68 wrong extension and wrong `Content-Type` ignored. N69 response headers are exactly the table (a golden test), run in proxy mode and in presigned mode. N70 video with trailing data; the server never decodes video. N71 font with bad tables. N72 font over size. N72a a guest cannot obtain an original. Fuzzing: image headers, font tables, container parsers. |
| Blobs | N73 expired grant. N74 changed `exp`. N75 changed surface, blob hash or session id. N76 grant for a tenant's blob used under another tenant. N77 request for a blob the surface does not reference. N77a a viewer is denied a blob referenced only by a deleted object, and allowed one in a version their role may open. N78 a viewer on surface A cannot get a grant for a blob that belongs only to surface B. N79 a change naming an ungranted hash is refused. N80 uploading a file that another tenant holds gives the same response as a new file. N80c an upload request (`POST`, `PATCH` or `HEAD`) after the link is turned off is refused. N80d an upload with more bytes than the grant's size, or a different blob, is refused. N80e after a restart (a new lease epoch), the video grant of a session revoked before the restart and an old upload grant are both refused, and a grant issued by the new incarnation works; the same holds after a room crash and after an idle unload; the endpoint reads the epoch from the durable lease, not from room memory. N80a turning a link off makes the next blob request of its guests fail (revoked-session list), for image and video grants alike. N80b a presigned storage address has a lifetime of at most 60 seconds and is not issued for a revoked session. |
| Privacy | N86 `locatable: false` refuses jump and shows no jump action. N87 no message to another client contains a view that was not requested (scan of all outputs in the presence tests). N88 jump rate limit. N89 follow notice is sent. N90 sign-out deletes IndexedDB and OPFS data in the browser test. |
| Client | N91 a name containing markup renders as text in canvas and mirror. N92 pasting HTML yields plain text only. N93 the web shell runs under the CSP with no violation report. N94 a build check that the shell uses no string-to-code API. |
| Supply chain | S1 `cargo deny check` (advisories, licenses, bans, sources). S2 `cargo audit` nightly. S3 `cargo vet` with no unaudited security-relevant crate. S4 a workflow lint that fails on an action not pinned to a SHA and on `pull_request_target` with a checkout of the pull request. S5 the double wasm build with equal hashes. |

## Dependencies

All versions are the latest seen on crates.io on 2026-11-11 (through its API). "Audit status" means a published
third-party security audit; none was verified for any crate below unless stated.

| Crate | Version | License | Use | Maintenance and audit |
|---|---|---|---|---|
| `jsonwebtoken` | 11.1.0 | MIT | Verify connect and admin tokens (server only) | Updated 2026-09-16; about 241 million downloads; backend choice `aws_lc_rs` or `rust_crypto` since 10.0; EdDSA supported; default `Validation` allows only HS256, so it is always built explicitly; a changelog line mentions a fix of type confusion for `exp`/`nbf`; no audit found (unverified) |
| `aws-lc-rs` | 1.19.0 | ISC AND (Apache-2.0 OR ISC) | Backend for `jsonwebtoken`, HMAC for blob grants, random bytes, TLS provider for `rustls` | Updated 2026-10-28; AWS-LC C code (it needs a C build step); FIPS-validated variants exist (unverified for this release). The C library `aws-lc-sys` may add the `OpenSSL` license to the list (unverified) |
| `ring` | 0.17.14 | Apache-2.0 AND ISC | Alternative to `aws-lc-rs` | Last release 2025-03-11, so the least active of the options; not chosen |
| `ed25519-dalek` | 3.0.0 | BSD-3-Clause | Only if a custom verifier is preferred over `jsonwebtoken` | Updated 2026-07-06; BSD-3-Clause is permissive and compatible; needs attribution in binary notices. Not chosen |
| `rustls` | 0.23.x stable (a 0.24 pre-release exists) | Apache-2.0 OR ISC OR MIT | TLS in the server | Updated 2026-10-14 for the pre-release; widely used; third-party audit exists (unverified here) |
| `governor` | 0.10.4 | MIT | GCRA rate limiting | Updated 2025-12-16; 93 million downloads |
| `unicode-normalization` | 0.1.26 | (MIT OR Apache-2.0) AND Unicode-3.0 | NFC for names | Updated 2026-10-16 |
| `unicode-segmentation` | version not checked (unverified) | MIT OR Apache-2.0 (unverified) | Grapheme-cluster counts for display names | Not checked; fetch of crates.io failed |
| `unicode-security` | 0.1.3 | MIT/Apache-2.0 | Confusable skeletons (UTS 39) | Updated 2026-11-02; small crate |
| `subtle` | 2.6.1 | BSD-3-Clause | Constant-time comparison where `aws-lc-rs` does not provide it | Last release 2024-06-24 (stable, small) |
| `getrandom` | 0.4.3 | MIT OR Apache-2.0 | Random secrets, if `aws-lc-rs` is not used for it | Updated 2026-06-17 |
| `pasetors` | 0.8.1 | MIT | Not used (alternative) | Updated 2026-08-30 |
| `biscuit-auth` | 7.0.0-alpha.1 | Apache-2.0 | Not used (alternative) | Pre-release only seen |
| `cargo-deny` | 0.20.2 | MIT OR Apache-2.0 | CI tool | Updated 2026-07-09 |
| `cargo-audit` | 0.23.0 | Apache-2.0 OR MIT | CI tool | Updated 2026-11-10 |
| `cargo-vet` | 0.10.3 | Apache-2.0 OR MIT | CI tool | Updated 2026-10-22 |

Image, font and container parsers belong to section 05. Every license above is compatible with MIT OR Apache-2.0:
permissive (MIT, BSD-3-Clause, ISC, Apache-2.0, Unicode-3.0). The tools are not linked into the product. If the
library is shipped with `aws-lc-rs`, its notices and the attribution clauses travel with the binary.

Why `jsonwebtoken` over a hand-written verifier on `ed25519-dalek`: it is widely reviewed and handles base64url,
JSON and claim checks; the risk of its algorithm agility is removed by always constructing `Validation` with
`EdDSA`. If the review finds a gap, a verifier of about 150 lines on `ed25519-dalek` is the fallback. The
choice between the `aws_lc_rs` and `rust_crypto` back ends is `aws_lc_rs`, so the server has one crypto library; the
wasm client never verifies tokens and links no crypto for them.

## What this assumes of other sections

| Section | Requirement |
|---|---|
| 01 positions | The encoded size (about 3.1 KB) and the level cap (+-1024) are readable from the raw bytes before any number is built. Decoding never allocates from an unvalidated length. Surface bounds are enforced by the server. |
| 02 document model | Object changes are the server's own typed format in a purpose-built object map sequenced by the server, so the server inspects every field and accepts or rejects a whole change. The server may author changes of its own (`DeleteBy`, pausing videos, rebalancing order keys); they carry `Actor::System`, are audited, and do not count against any guest budget. `DeleteBy` is owner-only; the role check and the member-only version operations are the ones in the matrix; guest `UserId` and `LinkId` are the `Principal` above. A CRDT library is used only for text inside text objects, behind a `TextDoc` trait; for text the server must still be able to tell, before applying, that an update touches only that text object (question 1). Every object records its creator (`Principal`). Group distance, text length and run counts are bounded in the model. Whether Loro or yrs can inspect a text update before applying it is unverified. |
| 03 sync, server, versions | Frames have a length prefix checked against the caps before reading the body. The server sends nothing before `auth_ok`. `Authz::check` is called for every message and HTTP request. A restore is an audited change by an authenticated principal, owner-only for now, never done by a guest. Bulk data is written in chunks of at most 16 KiB while unsent bytes stay under 64 KiB, so presence is not delayed. The room tells the blob store which sessions were revoked. View-scoped sync never sends an object outside the role's reach (there is none, but export and version opens are role-checked). Compression is off or bounded as above. Storage and indexes take a `TenantScope`. The order of the socket decides which messages precede a role change. |
| 04 presence | Routes only what the receiver's view contains; never sends `sub`, address, link id or another client's view unless a `JumpTo` was allowed; applies the presence rate limit before routing; honours `locatable`; drops under load rather than queues; colors are assigned by the server; names arrive validated; guests carry `kind: guest`. |
| 05 media | Implements the media table: type allow-list; the media worker as a separate process with memory and CPU limits; the server never decodes video; derived files displayed, originals download-only for members; the grant, headers and separate site above; 24-hour cleanup; per-tenant dedup; server-computed hashes; no SVG; no animated images. |
| 06 rendering and text | Draws names and text itself; uses no platform font parser on untrusted files; handles every number it gets as hostile (finite, bounded). |
| 07 shell, controls, accessibility | The mirror sets text with `textContent` or similar, never markup; clipboard paste takes plain text and images only; the web shell runs under the recommended CSP with no string-to-code APIs; `join_with_link` implements the exchange and removes the fragment at once; "leave" and "sign out" clear local storage keyed by principal; controls the role cannot use are hidden or disabled (convenience only); the core is single-threaded, with no wasm threads. |

## Questions for the owner

These are product decisions, or depend on one reserved for the owner.

1. **CRDT library for text (reserved).** The proposed merge model (section 02) keeps a CRDT library only for text
   inside text objects, so the server inspects every object change itself. Security still needs one thing from the
   library: the server must be able to check, before applying a text update, that it touches only the text object
   the sender may edit. Recommendation: put "inspect before apply" in the spike for the text library; if it cannot,
   the server applies the update to a scratch copy and compares.
2. **Default expiry of share links.** Recommended: 30 days, with "never" allowed unless the tenant forbids it.
   Alternative: no default expiry (easier for a classroom, worse for forgotten links). Which is the product wish?
3. **What guests may upload.** Recommended: Guest-E images only (8 MiB), no video, no fonts. Allowing video for
   guests raises the platform-decoder risk.
4. **May viewers export the whole surface?** Recommended: off by default, a tenant setting. A viewer can already see
   everything by panning, but export makes copying one click.
5. **`locatable` default and presenter mode.** Recommended: on by default (as the high-level design accepts) with a
   per-person switch. Should an owner be able to say "presenters cannot hide"? And should guests default to off?
6. **Deletion grace period.** Recommended: immediate deletion for the owner with a typed confirmation, and no undo.
   Alternative: 7 days in a trash state that still costs storage and keeps content the owner meant to delete.
7. **Application-level encryption at rest per tenant.** Recommended: not in version 1; storage-layer encryption by
   the operator. It protects stolen backups only.
8. **Guest names: ban on the first offence, or only link-level remedies?** Recommended: no name word lists in the
   core, owner rename and remove, operator hook for lists.
9. **Whether to require embedders to use Ed25519.** Recommended: yes, one algorithm. It costs embedders on
   platforms without EdDSA a small library.
10. **Who may restore versions.** Until you decide, restore is owner-only (members only, never guests). The
    alternative is owner and editor members. A restore discards later work, so owner-only is safer and costs an
    editor having to ask. The high-level design said "editor role".
11. **The first app (reserved).** Security impact: a classroom or meeting tool wants many guests behind one
    address and open links; an enterprise tool wants none. The defaults above lean toward the first; recommended to
    decide with the first app.
12. **Publishing, domains and the public repository** stay the owner's (AGENTS.md). Provenance and attestation steps
    need the repository to be public or the plan to support them (unverified).

## Sources

All fetched on 2026-11-11 unless noted.

- Liveblocks, Authentication: https://liveblocks.io/docs/authentication
- Liveblocks, Access token permissions: https://liveblocks.io/docs/rooms/authentication/access-token-permissions
- tldraw, Sync: https://tldraw.dev/docs/sync
- PartyKit, Authentication: https://docs.partykit.io/guides/authentication/
- y-sweet repository: https://github.com/jamsocket/y-sweet (docs page `.../concepts/authentication.md` returned 404)
- Figma, Share files and prototypes:
  https://help.figma.com/hc/en-us/articles/360040531773-Share-files-and-prototypes
- Google Drive API, Manage sharing: https://developers.google.com/workspace/drive/api/guides/manage-sharing
- Excalidraw, End-to-End Encryption in the Browser: https://plus.excalidraw.com/blog/end-to-end-encryption (found by
  search; the page itself was not opened, the link format comes from search snippets and a fork README, unverified)
- W3C TAG, Good Practices for Capability URLs: https://www.w3.org/2001/tag/doc/capability-urls/
- RFC 8725, JWT Best Current Practices: https://www.rfc-editor.org/rfc/rfc8725.html
- RFC 6455, The WebSocket Protocol (sections 1.6, 4.2.2, 5.1 read; 10.2 and 10.3 not read):
  https://datatracker.ietf.org/doc/html/rfc6455
- OWASP, WebSocket Security Cheat Sheet:
  https://cheatsheetseries.owasp.org/cheatsheets/WebSocket_Security_Cheat_Sheet.html
- OWASP, File Upload Cheat Sheet: https://cheatsheetseries.owasp.org/cheatsheets/File_Upload_Cheat_Sheet.html
- OWASP ASVS 5.0.0: https://asvs.owasp.org/ and chapter list at https://github.com/OWASP/ASVS/tree/v5.0.0/5.0/en
- Biscuit introduction: https://doc.biscuitsec.org/getting-started/introduction
- PASETO specification repository: https://github.com/paseto-standard/paseto-spec
- `pasetors` documentation: https://docs.rs/pasetors/latest/pasetors/
- `jsonwebtoken` README and changelog: https://github.com/Keats/jsonwebtoken
- MDN, CSP `script-src` (`'wasm-unsafe-eval'`):
  https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/script-src
- web.dev, COOP and COEP: https://web.dev/articles/coop-coep
- cargo-deny: https://embarkstudios.github.io/cargo-deny/
- cargo-vet: https://github.com/mozilla/cargo-vet
- RustSec advisory database: https://github.com/RustSec/advisory-db
- crates.io API for the crates in Dependencies, e.g. https://crates.io/api/v1/crates/jsonwebtoken (and
  `ed25519-dalek`, `pasetors`, `biscuit-auth`, `aws-lc-rs`, `ring`, `governor`, `rustls`, `cargo-deny`, `cargo-vet`,
  `cargo-audit`, `unicode-security`, `unicode-normalization`, `subtle`, `getrandom`)

Not verified, listed together: Miro's sharing model; Figma's socket authentication; y-sweet token expiry and levels;
Liveblocks token lifetime; any third-party audit of a crate named above; GCRA being governor's algorithm; the
`OpenSSL` license in `aws-lc-sys`; Loro and yrs ability to inspect an update before applying it; reproducibility of
the `wasm-bindgen` plus `wasm-opt` chain; Trusted Types compatibility of generated glue; Google's separate content
domain; backup retention norms; legal sufficiency of the privacy design; which publishers' `cargo-vet` audits can be
imported; GitHub or Sigstore attestation details; EdDSA support in particular embedder languages.
