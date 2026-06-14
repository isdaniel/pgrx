# Plan B Design — `PgBoxIn<'mcx, T, A>` Lifetime-Bound Sibling of `PgBox`

**Date:** 2026-06-14
**Status:** Spec for review
**Scope decision:** B3 (two-stage: ship sibling type first, evaluate consolidation later) + S1 (minimum viable surface)
**Related upstream:** issue #2204 (`PgBox` is unsound re: lifetimes), PR #2210 (introduced `MemCx` / `PBox`), TODO at `pgrx/src/pgbox.rs:95`

---

## 1. Goal

Close the soundness gap between `PgBox<T, A>` (no `'mcx` lifetime → cross-context use-after-free is invisible to the borrow checker) and the post-#2210 lifetime-aware story (`MemCx<'mcx>`, `PBox<'mcx, T>`), **without** breaking any existing `PgBox` call-site.

We do this by introducing a sibling type `PgBoxIn<'mcx, T, A>` that:

- carries the `'mcx` brand `PBox` already uses,
- preserves the `WhoAllocated` typestate `PgBox` already uses,
- exposes the smallest API surface that lets users prove the value (`from_pg_in`, `alloc_in`, `alloc0_in`, plus `Deref`/`DerefMut`/`Drop`/`null`/`is_null`/`as_ptr`/`into_pg`/`into_pg_boxed`).

Future PRs can extend `PgBoxIn` with `SqlTranslatable`, `BoxRet`, `ArgAbi`, and the remaining `alloc_*` variants, or merge `'mcx` back into `PgBox` itself with `'mcx = 'static` default.

## 2. Non-Goals

- **No change to `PgBox`.** No new methods, no `#[deprecated]`, no doc tweaks beyond a single "see also" cross-link. This keeps the PR diff small and the review surface narrow.
- **No `SqlTranslatable` / `BoxRet` / `ArgAbi`.** `PgBoxIn` is not yet usable as a `#[pg_extern]` argument or return type. Users at the FFI boundary call `into_pg` or `into_pg_boxed` to cross.
- **No `alloc_in_context_in` or `alloc_node_in`.** The `PgMemoryContexts`-based and `PgNode`-based allocation paths can be added in a follow-up once the core type is accepted.
- **No reuse of `prelude`.** `PgBoxIn` is opt-in via `pgrx::pgbox::PgBoxIn`. Existing tutorials and examples are untouched.

## 3. Architecture

### 3.1 File layout

`pgrx/src/pgbox.rs` becomes a directory:

```
pgrx/src/
├── pgbox/
│   ├── mod.rs       (current pgbox.rs content, unchanged except `mod in_mcx;`)
│   └── in_mcx.rs    (NEW — PgBoxIn definition + impls + unit tests)
```

Rationale: keeps `WhoAllocated` typestate concentrated in one module, avoids polluting `palloc/pbox.rs` (which deliberately stays narrow around `PBox`'s single Drop invariant).

### 3.2 Type definition

```rust
// pgrx/src/pgbox/in_mcx.rs
use crate::memcx::MemCx;
use crate::pg_sys;
use crate::pgbox::{AllocatedByPostgres, AllocatedByRust, PgBox, WhoAllocated};
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

#[repr(transparent)]
pub struct PgBoxIn<'mcx, T, AllocatedBy: WhoAllocated = AllocatedByPostgres> {
    ptr: Option<NonNull<T>>,
    _cx: PhantomData<&'mcx MemCx<'mcx>>,
    _alloc: PhantomData<AllocatedBy>,
}
```

Key invariants:

- `#[repr(transparent)]` over `Option<NonNull<T>>` — same layout as `PgBox`, ABI-compatible, leaves the door open to merge into `PgBox` later (B2 in the analysis doc).
- `PhantomData<&'mcx MemCx<'mcx>>` makes the type **invariant** in `'mcx` — borrowed pointers cannot be silently widened.
- `WhoAllocated` typestate is reused unchanged. Drop semantics are inherited via `A::maybe_pfree`.

### 3.3 Public API surface (S1 minimum)

```rust
// Constructors
impl<'mcx, T> PgBoxIn<'mcx, T, AllocatedByPostgres> {
    /// # Safety
    /// `ptr` must be a valid Postgres-owned pointer that lives at least as long as `_memcx`.
    pub unsafe fn from_pg_in(ptr: *mut T, _memcx: &MemCx<'mcx>) -> Self;
}

impl<'mcx, T> PgBoxIn<'mcx, T, AllocatedByRust> {
    /// SAFE — lifetime is tracked, replacing the unsafety justification on `PgBox::alloc`.
    pub fn alloc_in(memcx: &MemCx<'mcx>) -> Self;
    pub fn alloc0_in(memcx: &MemCx<'mcx>) -> Self;
}

// Common methods
impl<'mcx, T, A: WhoAllocated> PgBoxIn<'mcx, T, A> {
    pub fn null() -> Self;
    pub fn is_null(&self) -> bool;
    pub fn as_ptr(&self) -> *mut T;

    /// Hand the raw pointer back to Postgres, dropping the `'mcx` brand.
    pub fn into_pg(self) -> *mut T;

    /// Convert to the legacy un-lifetimed `PgBox` for FFI hand-off into existing API.
    pub fn into_pg_boxed(self) -> PgBox<T, AllocatedByPostgres>;
}

// Trait impls
impl<'mcx, T, A: WhoAllocated> Deref for PgBoxIn<'mcx, T, A> { /* panics on null, like PgBox */ }
impl<'mcx, T, A: WhoAllocated> DerefMut for PgBoxIn<'mcx, T, A> { /* same */ }
impl<'mcx, T, A: WhoAllocated> Drop for PgBoxIn<'mcx, T, A> {
    fn drop(&mut self) {
        if let Some(ptr) = self.ptr {
            unsafe { A::maybe_pfree(ptr.as_ptr().cast()); }
        }
    }
}
```

### 3.4 What deliberately stays out of S1

`Clone`, `Debug`, `Display`, `PartialEq`/`Eq`, `with()`, `alloc_in_context_in`, `alloc_node_in`, `SqlTranslatable`, `BoxRet`, `ArgAbi`. Each is mechanical to add later and would inflate the first PR without changing the soundness story.

## 4. Soundness Argument

The `'mcx` brand on `PgBoxIn` is the same brand `PBox` already uses, so the soundness reasoning is identical:

1. `MemCx<'mcx>` is borrowed-from for every constructor — the returned `PgBoxIn<'mcx, …>` cannot outlive the `&MemCx<'mcx>` it was built with, by Rust's lifetime rules.
2. `current_context(|cx: &MemCx<'curr>| { ... })` ensures the brand is tied to a real, currently-live MemoryContext at runtime.
3. `Drop` honors `WhoAllocated::maybe_pfree`, so:
   - `AllocatedByRust` always `pfree`s — no leak.
   - `AllocatedByPostgres` never `pfree`s — no double-free.
4. `into_pg` / `into_pg_boxed` consume `self`, suppressing Drop, so handing ownership back to Postgres cannot leak or double-free.
5. `from_pg_in` is `unsafe` because the FFI invariant ("pointer truly belongs to `memcx`") cannot be statically checked.

The above is the same shape `PBox` ships with today; no new unsafe reasoning is introduced.

## 5. Testing Plan

### 5.1 Unit tests (`pgrx/src/pgbox/in_mcx.rs`)

| Test | What it proves |
|---|---|
| `from_pg_in_null_is_null` | constructor handles null |
| `from_pg_in_non_null_round_trip` | `as_ptr() == input ptr` |
| `into_pg_returns_same_ptr` | `into_pg` does not transform the pointer |
| `null_is_null` | `null()` returns `is_null() == true` |

These run without a Postgres backend (use raw `&mut value as *mut T` like the existing `PgBox` tests).

### 5.2 Compile-fail doctest

On the type-level doc comment of `PgBoxIn`:

```rust
/// ```compile_fail,E0597
/// use pgrx::memcx;
/// use pgrx::pgbox::PgBoxIn;
/// let escaped = memcx::current_context(|cx| {
///     unsafe { PgBoxIn::<i32, _>::from_pg_in(std::ptr::null_mut(), cx) }
/// });
/// let _ = escaped.is_null();
/// ```
```

This test is the headline soundness proof — it shows that the lifetime brand actually rejects the bug class the analysis report describes.

### 5.3 Integration tests (`pgrx-tests/src/tests/pgbox_in_tests.rs`, new file)

Run inside an actual PG backend:

| Test | What it proves |
|---|---|
| `alloc_in_drops_pfree` | `alloc_in` followed by Drop reduces context-allocated bytes (uses `MemoryContextMemAllocated`) |
| `alloc0_in_zero_filled` | memory is zeroed |
| `into_pg_then_drop_no_double_free` | `into_pg` followed by manual `pfree` does not crash |
| `from_pg_in_drop_does_not_free` | wrapping a Postgres-owned pointer and dropping leaves it intact (subsequent dereferences from the same context still succeed) |

These follow the conventions of existing files under `pgrx-tests/src/tests/`.

### 5.4 Example crate (`pgrx-examples/pgbox_in_demo/`, new directory)

A minimal extension with one `#[pg_extern]` `pgbox_in_demo()` that:

1. Calls `current_context(|cx| { let mut b = PgBoxIn::<MyStruct, _>::alloc_in(cx); ...; b.into_pg_boxed() })` — demonstrates the safe-allocation path.
2. Wraps a `pg_sys::relation_open()` result with `from_pg_in`, uses it inside a closure, and (in commented form) shows the `compile_fail` if you tried to leak the box out.
3. Has a `lib.rs` doctest that mirrors the `compile_fail` proof for users browsing on docs.rs.

The README of the example explicitly contrasts `PgBox::alloc()` (unsafe) with `PgBoxIn::alloc_in()` (safe) as a migration guide.

## 6. Verification Checklist (against original prompt)

| Requirement | How verified |
|---|---|
| No memory leak | Integration test compares `MemoryContextMemAllocated` before/after Drop |
| No unsoundness | `compile_fail` doctest + reuse of `PBox`'s established brand |
| Unit tests | §5.1 — 4 tests |
| Integration tests | §5.3 — 4 tests |
| Real-extension example | §5.4 — `pgbox_in_demo` |
| Build all pass | `cargo +nightly check --all-features --all-targets` |
| All unit tests pass | `cargo pgrx test pg17` (and matrix in CI) |

## 7. Open Questions for Maintainers (PR description)

1. **Naming**: `PgBoxIn` (mirrors `Box::new_in`) vs `ScopedPgBox` / `BoundPgBox`. Open to bikeshed.
2. **Two-stage roadmap**: is the team open to a follow-up PR that consolidates `'mcx` back into `PgBox` itself with `'mcx = 'static` default once `PgBoxIn` proves itself?
3. **Direction of `into_pg_boxed`**: should we additionally provide `From<PgBox<T, AllocatedByPostgres>> for PgBoxIn<'static, T, AllocatedByPostgres>` to ease incremental opt-in? Deliberately deferred from S1.
4. **Module placement**: keep the new type at `pgrx::pgbox::PgBoxIn`, or move both `PgBox` and `PgBoxIn` under `pgrx::palloc::` to sit next to `PBox`? S1 chooses the former for minimal churn.

## 8. Out of Scope (explicit)

- `#[pg_extern]` integration (needs `SqlTranslatable` / `BoxRet` / `ArgAbi`).
- Migration of internal pgrx call-sites of `PgBox` to `PgBoxIn`.
- Deprecation of `PgBox`.
- `Clone` / `Debug` / `Display` / `PartialEq` / `Eq`.

Each is tracked as a follow-up; none is required for the soundness win.
