---
id: 01M63A9ZS73ND4VBRAJW27K9NR
title: "ADR 0002: Envelope encryption format and key providers"
status: verified
owner: "@cruisinme30"
tags: [adr, security, foundation]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D34, D35, D36

:::

## Context

The brief (section 11) makes envelope encryption at rest the default:
XChaCha20-Poly1305 data-encryption keys (DEKs) per document, wrapped by a
key-encryption key (KEK) from the OS keychain, a KMS or Vault, or an
Argon2id passphrase, with rotation by re-wrapping. This ADR fixes the
on-disk format and the provider contract in `vellum-crypto`.

## Decisions

### D34: one fresh DEK per stored object, header in the object

- Every object written to a blob store gets its own random 256-bit DEK and
  a random 192-bit nonce. A document's file and each of its versions are
  separate objects, so this is at least as fine-grained as "per document"
  and needs no DEK lookup table.
- The wrapped DEK and the KEK id travel in a header at the start of the
  object: magic `\0VLE`, version, KEK id, wrapped DEK, nonce, ciphertext.
  A Markdown file never starts with a NUL byte, so plaintext and sealed
  objects are told apart without metadata.
- Consequence: deleting one object's header shreds that object, and a
  backup restores keys and data together.

### D35: the payload is bound to neither the path nor the header

- The payload's associated data is a fixed label. Binding it to the path
  would break renames (file names are title slugs); binding it to the
  header would force re-encrypting every payload on rotation.
- Rotation (`rewrap`) unwraps the DEK with the old KEK, wraps it with the
  current one and rewrites only the header. Retired KEKs stay in the
  keyring until no header names them.
- Consequence: someone with write access to the store could swap two whole
  sealed objects. That is an integrity threat to the store itself, covered
  by authorization, the audit log and the index's content hashes, not by
  encryption at rest.

### D36: providers own the KEK and the wrapped form

- `KeyProvider` is async and treats the wrapped DEK as opaque bytes, so a
  KMS or Vault transit adapter wraps remotely and the KEK never leaves it.
- `LocalKeyring` serves the OS keychain and passphrase cases. A passphrase
  KEK id carries its Argon2id salt and costs, so the passphrase alone
  re-derives the KEK from any envelope header.
