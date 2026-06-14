# PgBoxIn Plan B Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `PgBoxIn<'mcx, T, A>` — a lifetime-bound sibling of `PgBox<T, A>` — so `MemCx`-allocated values are protected from cross-context use-after-free at compile time. Zero breakage to existing `PgBox` users.

**Architecture:** New `pgrx::pgbox::PgBoxIn` reuses the `WhoAllocated` typestate of `PgBox` and the `'mcx` brand of `PBox`/`MemCx`. S1 minimum surface only: 3 constructors (`from_pg_in`, `alloc_in`, `alloc0_in`), `null`/`is_null`/`as_ptr`, `into_pg`/`into_pg_boxed`, `Deref`/`DerefMut`/`Drop`. No `SqlTranslatable`, no `BoxRet`, no `Clone`/`Debug`/`PartialEq` — all deferred.

**Tech Stack:** Rust 2024 edition, MSRV 1.96, `pgrx` workspace. Tests use both `cargo test` (no PG backend) and `#[pg_test]` (inside a real Postgres backend).

**Spec:** `docs/superpowers/specs/2026-06-14-pgbox-plan-b-design.md`

---

## File Structure

| Path | Action | Responsibility |
|---|---|---|
| `pgrx/src/pgbox.rs` | **Move** to `pgrx/src/pgbox/mod.rs` (no content change) | Hosts existing `PgBox` + new `mod in_mcx;` declaration |
| `pgrx/src/pgbox/in_mcx.rs` | **Create** | `PgBoxIn` definition, all impls, unit tests |
| `pgrx/src/lib.rs` | No change required (`pub mod pgbox;` already covers the directory) | — |
| `pgrx-examples/pgbox_in_demo/Cargo.toml` | **Create** | Example crate manifest |
| `pgrx-examples/pgbox_in_demo/src/lib.rs` | **Create** | `#[pg_extern]` demo + `#[pg_test]` integration tests |
| `pgrx-examples/pgbox_in_demo/pgbox_in_demo.control` | **Create** | Postgres extension control file |
| `pgrx-examples/pgbox_in_demo/README.md` | **Create** | Migration-guide-style readme |

The `pgrx-examples/*` glob in the workspace root `Cargo.toml` (line ~5) picks up the new crate automatically — no workspace edit needed.

---

## Task 1: Bootstrap — split `pgbox.rs` to a directory module

**Files:**
- Move: `pgrx/src/pgbox.rs` → `pgrx/src/pgbox/mod.rs`
- Create (empty stub): `pgrx/src/pgbox/in_mcx.rs`
- Modify: `pgrx/src/pgbox/mod.rs` — add `mod in_mcx; pub use in_mcx::PgBoxIn;`

- [ ] **Step 1: Move the existing file**

```bash
cd /home/azureuser/pgrx
git mv pgrx/src/pgbox.rs pgrx/src/pgbox/mod.rs
```

- [ ] **Step 2: Create the empty submodule**

Create `pgrx/src/pgbox/in_mcx.rs` with:

```rust
//LICENSE Portions Copyright 2019-2021 ZomboDB, LLC.
//LICENSE
//LICENSE Portions Copyright 2021-2023 Technology Concepts & Design, Inc.
//LICENSE
//LICENSE Portions Copyright 2023-2023 PgCentral Foundation, Inc. <contact@pgcentral.org>
//LICENSE
//LICENSE All rights reserved.
//LICENSE
//LICENSE Use of this source code is governed by the MIT license that can be found in the LICENSE file.

//! Lifetime-bound sibling of [`PgBox`](super::PgBox).
//!
//! See `docs/superpowers/specs/2026-06-14-pgbox-plan-b-design.md`.
```

- [ ] **Step 3: Wire it into `pgbox/mod.rs`**

In `pgrx/src/pgbox/mod.rs`, just **after** the existing `use std::ptr::NonNull;` line (around line 19 of the original file), add:

```rust
mod in_mcx;
pub use in_mcx::PgBoxIn;
```

- [ ] **Step 4: Verify the workspace still builds**

```bash
cargo +nightly check -p pgrx --features pg17
```

Expected: clean compile, possibly a warning about unused `in_mcx` (acceptable at this step).

- [ ] **Step 5: Commit**

```bash
git add pgrx/src/pgbox/
git commit -m "refactor(pgbox): split pgbox.rs into directory module to host PgBoxIn"
```

---

## Task 2: Define `PgBoxIn` type + `null` / `is_null` / `as_ptr`

**Files:**
- Modify: `pgrx/src/pgbox/in_mcx.rs`

- [ ] **Step 1: Write the failing tests**

Append to `pgrx/src/pgbox/in_mcx.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pgbox::AllocatedByPostgres;

    #[test]
    fn null_is_null() {
        let b: PgBoxIn<'_, i32, AllocatedByPostgres> = PgBoxIn::null();
        assert!(b.is_null());
        assert!(b.as_ptr().is_null());
    }

    #[test]
    fn from_pg_in_non_null_round_trips() {
        let mut value: i32 = 42;
        let ptr = &mut value as *mut i32;
        // We do NOT enter a real MemCx here — this is a pure pointer-plumbing test.
        // Soundness of crossing MemCx boundaries is the job of the doctest in Task 8
        // and the #[pg_test] integration tests in Task 10.
        let memcx_stub = unsafe { crate::memcx::MemCx::from_ptr(core::ptr::null_mut()) };
        // SAFETY: we never call any MemCx method that dereferences the inner ptr in this test.
        let _ = &memcx_stub; // silence unused
        // Build directly using the private fields via `null()` then a manual mutation
        // is not possible. We instead trust `from_pg_in` which is exercised in Task 4.
    }
}
```

> **Note:** the second test stub above is intentionally minimal here — the real round-trip test lives in Task 4 once `from_pg_in` exists. We add it now only to keep the test module compiling.

Replace the second test with a simpler placeholder for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pgbox::AllocatedByPostgres;

    #[test]
    fn null_is_null() {
        let b: PgBoxIn<'_, i32, AllocatedByPostgres> = PgBoxIn::null();
        assert!(b.is_null());
        assert!(b.as_ptr().is_null());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx::tests::null_is_null
```

Expected: FAIL with `cannot find type 'PgBoxIn' in this scope`.

- [ ] **Step 3: Write the type and the three methods**

Append to `pgrx/src/pgbox/in_mcx.rs` (above the `tests` module):

```rust
use crate::memcx::MemCx;
use crate::pgbox::{AllocatedByPostgres, WhoAllocated};
use core::marker::PhantomData;
use core::ptr::NonNull;

/// Similar to [`PgBox`](super::PgBox), but the wrapped pointer's lifetime is tied
/// to a [`MemCx<'mcx>`].
///
/// This means the borrow checker enforces that the box cannot outlive the
/// memory context the pointer was allocated in — closing the cross-context
/// use-after-free hole that `PgBox` has by design.
///
/// See the spec at `docs/superpowers/specs/2026-06-14-pgbox-plan-b-design.md`
/// for the full motivation and roadmap.
#[repr(transparent)]
pub struct PgBoxIn<'mcx, T, AllocatedBy: WhoAllocated = AllocatedByPostgres> {
    ptr: Option<NonNull<T>>,
    _cx: PhantomData<&'mcx MemCx<'mcx>>,
    _alloc: PhantomData<AllocatedBy>,
}

impl<'mcx, T, A: WhoAllocated> PgBoxIn<'mcx, T, A> {
    /// Box nothing.
    #[inline]
    pub fn null() -> Self {
        PgBoxIn { ptr: None, _cx: PhantomData, _alloc: PhantomData }
    }

    /// Are we boxing a null pointer?
    #[inline]
    pub fn is_null(&self) -> bool {
        self.ptr.is_none()
    }

    /// Return the boxed pointer for hand-off to a Postgres C function.
    #[inline]
    pub fn as_ptr(&self) -> *mut T {
        match self.ptr {
            Some(p) => p.as_ptr(),
            None => core::ptr::null_mut(),
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx::tests::null_is_null
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add pgrx/src/pgbox/in_mcx.rs
git commit -m "feat(pgbox): introduce PgBoxIn type with null/is_null/as_ptr"
```

---

## Task 3: Implement `Drop` honoring `WhoAllocated`

**Files:**
- Modify: `pgrx/src/pgbox/in_mcx.rs`

- [ ] **Step 1: Write the failing test**

Inside the `tests` module in `pgrx/src/pgbox/in_mcx.rs`, append:

```rust
    /// Verifies that `AllocatedByPostgres` does NOT free on Drop.
    /// We construct over a stack int — if Drop attempted `pfree`, the test process
    /// would crash (since pfree on a stack pointer is undefined in PG, but here we
    /// just rely on the fact that maybe_pfree is a no-op for AllocatedByPostgres).
    #[test]
    fn drop_allocated_by_postgres_is_noop() {
        let mut value: i32 = 7;
        let ptr = &mut value as *mut i32;
        // Manually construct without using from_pg_in (not yet implemented).
        let b: PgBoxIn<'_, i32, AllocatedByPostgres> = PgBoxIn {
            ptr: NonNull::new(ptr),
            _cx: PhantomData,
            _alloc: PhantomData,
        };
        drop(b);
        // If we reach here, Drop was a no-op as expected.
        assert_eq!(value, 7);
    }
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx::tests::drop_allocated_by_postgres_is_noop
```

Expected: FAIL with `error[E0117]` or similar — but actually the struct fields are private, so the construction won't compile from inside `tests` module. **The test module is `mod tests` *inside* `in_mcx.rs`, which has access to private fields**, so it should compile. If it does compile (because the struct is also defined in this file), expected: FAIL with "missing trait Drop" — wait, Drop is auto-no-op without explicit impl. Reread:

The test asserts `value == 7` after Drop. Without an explicit `Drop` impl, this passes trivially. So this test is **not yet failing**. We need a stronger test.

Replace the test above with:

```rust
    use core::sync::atomic::{AtomicUsize, Ordering};

    static FAKE_PFREE_CALLS: AtomicUsize = AtomicUsize::new(0);

    struct AllocatedByCounter;
    unsafe impl WhoAllocated for AllocatedByCounter {
        unsafe fn maybe_pfree(_ptr: *mut std::os::raw::c_void) {
            FAKE_PFREE_CALLS.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn drop_calls_who_allocated_pfree() {
        FAKE_PFREE_CALLS.store(0, Ordering::SeqCst);
        let mut value: i32 = 7;
        let ptr = &mut value as *mut i32;
        {
            let _b: PgBoxIn<'_, i32, AllocatedByCounter> = PgBoxIn {
                ptr: NonNull::new(ptr),
                _cx: PhantomData,
                _alloc: PhantomData,
            };
            assert_eq!(FAKE_PFREE_CALLS.load(Ordering::SeqCst), 0);
        }
        assert_eq!(FAKE_PFREE_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn drop_null_does_not_call_pfree() {
        FAKE_PFREE_CALLS.store(0, Ordering::SeqCst);
        {
            let _b: PgBoxIn<'_, i32, AllocatedByCounter> = PgBoxIn::null();
        }
        assert_eq!(FAKE_PFREE_CALLS.load(Ordering::SeqCst), 0);
    }
```

> **Note:** `WhoAllocated::maybe_pfree` is currently declared `unsafe fn maybe_pfree(...)` (no `unsafe` keyword on the trait itself — check `pgrx/src/pgbox/mod.rs:100-108`). If it is `unsafe trait`, prepend `unsafe` to `impl`. Verify with `grep -n "trait WhoAllocated" pgrx/src/pgbox/mod.rs`.

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx::tests::drop_calls_who_allocated_pfree
```

Expected: FAIL with `expected 1 calls, got 0` (because no `Drop` impl yet).

- [ ] **Step 3: Implement `Drop`**

Append to `pgrx/src/pgbox/in_mcx.rs` (outside the `tests` module):

```rust
impl<'mcx, T, A: WhoAllocated> Drop for PgBoxIn<'mcx, T, A> {
    fn drop(&mut self) {
        if let Some(ptr) = self.ptr {
            // SAFETY: `WhoAllocated::maybe_pfree` is the trait's contract: each
            // implementor decides whether the pointer should be freed. The pointer
            // was either supplied by the caller (and asserted valid via the
            // `unsafe` constructors) or produced by our own `alloc_in` family.
            unsafe { A::maybe_pfree(ptr.as_ptr().cast()); }
        }
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx::tests
```

Expected: all 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add pgrx/src/pgbox/in_mcx.rs
git commit -m "feat(pgbox): PgBoxIn Drop honors WhoAllocated::maybe_pfree"
```

---

## Task 4: Implement `from_pg_in`

**Files:**
- Modify: `pgrx/src/pgbox/in_mcx.rs`

- [ ] **Step 1: Write the failing test**

Inside the `tests` module, append:

```rust
    #[test]
    fn from_pg_in_round_trips_pointer() {
        let mut value: i32 = 99;
        let ptr = &mut value as *mut i32;
        // SAFETY: ptr is valid for the duration of this test; the MemCx is a stub
        // since we do not call any MemCx method that dereferences its inner pointer.
        let memcx = unsafe { crate::memcx::MemCx::from_ptr(core::ptr::dangling_mut()) };
        let b: PgBoxIn<'_, i32, AllocatedByPostgres> =
            unsafe { PgBoxIn::from_pg_in(ptr, &memcx) };
        assert!(!b.is_null());
        assert_eq!(b.as_ptr(), ptr);
    }

    #[test]
    fn from_pg_in_null_input_is_null() {
        let memcx = unsafe { crate::memcx::MemCx::from_ptr(core::ptr::dangling_mut()) };
        let b: PgBoxIn<'_, i32, AllocatedByPostgres> =
            unsafe { PgBoxIn::from_pg_in(core::ptr::null_mut(), &memcx) };
        assert!(b.is_null());
    }
```

> **Note:** `MemCx::from_ptr` is `pub(crate)` (see `pgrx/src/memcx.rs:32`). Since this test lives inside the `pgrx` crate, that visibility is sufficient. `core::ptr::dangling_mut()` is a stable function from Rust 1.84+; if MSRV 1.96 covers it (it does), use it. The `MemCx::from_ptr` panics on null, so we must use a non-null sentinel — `dangling_mut()` produces a non-null but unaligned/invalid pointer that's never dereferenced.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx::tests::from_pg_in_round_trips_pointer
```

Expected: FAIL with `no function or associated item named 'from_pg_in'`.

- [ ] **Step 3: Implement `from_pg_in`**

Append to `pgrx/src/pgbox/in_mcx.rs` (above the `tests` module):

```rust
impl<'mcx, T> PgBoxIn<'mcx, T, AllocatedByPostgres> {
    /// Wrap a Postgres-allocated pointer that lives in `_memcx`.
    ///
    /// When this `PgBoxIn` is dropped, the wrapped memory is **not** freed —
    /// since Postgres allocated it, Postgres owns its lifetime.
    ///
    /// # Safety
    /// - `ptr` must either be null or a valid pointer to a `T` allocated within
    ///   the memory context represented by `_memcx` (or a parent thereof).
    /// - The `T` value pointed to must be valid for reads and writes for the
    ///   duration of `'mcx`.
    #[inline]
    pub unsafe fn from_pg_in(ptr: *mut T, _memcx: &MemCx<'mcx>) -> Self {
        PgBoxIn { ptr: NonNull::new(ptr), _cx: PhantomData, _alloc: PhantomData }
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx::tests
```

Expected: all 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add pgrx/src/pgbox/in_mcx.rs
git commit -m "feat(pgbox): PgBoxIn::from_pg_in wraps a MemCx-bound Postgres pointer"
```

---

## Task 5: Implement `alloc_in` and `alloc0_in` (safe constructors)

**Files:**
- Modify: `pgrx/src/pgbox/in_mcx.rs`

> **Important:** these constructors call `MemCx::alloc_bytes` / `alloc_zeroed_bytes`, which call into Postgres' `MemoryContextAllocExtended`. Calling these without a real PG backend will not link cleanly in unit tests. We therefore add **only a compilation-shape test** here. Runtime behaviour is verified in the `#[pg_test]` integration tests (Task 10).

- [ ] **Step 1: Write the failing compilation test**

Inside the `tests` module, append:

```rust
    /// Compile-time-only check: the `alloc_in` and `alloc0_in` signatures must
    /// be `safe` (no `unsafe` keyword) and must produce a `PgBoxIn` carrying
    /// the input `MemCx`'s lifetime.
    #[allow(dead_code)]
    fn _compile_check_alloc_in_signatures<'mcx>(cx: &crate::memcx::MemCx<'mcx>) {
        let _: PgBoxIn<'mcx, i32, crate::pgbox::AllocatedByRust> = PgBoxIn::alloc_in(cx);
        let _: PgBoxIn<'mcx, i32, crate::pgbox::AllocatedByRust> = PgBoxIn::alloc0_in(cx);
    }
```

- [ ] **Step 2: Run test to verify it fails to compile**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx
```

Expected: FAIL with `no function or associated item named 'alloc_in'`.

- [ ] **Step 3: Implement `alloc_in` and `alloc0_in`**

Append to `pgrx/src/pgbox/in_mcx.rs` (above the `tests` module):

```rust
use crate::pgbox::AllocatedByRust;

impl<'mcx, T> PgBoxIn<'mcx, T, AllocatedByRust> {
    /// Allocate enough memory for a `T` inside `memcx`. The memory is
    /// uninitialized — the caller must initialize it before reading.
    ///
    /// Unlike [`PgBox::alloc`](super::PgBox::alloc), this function is
    /// **safe**: the returned `PgBoxIn<'mcx, T, AllocatedByRust>` cannot
    /// outlive `memcx`, so the memory-context-deletion-after-use UAF that
    /// motivated `PgBox::alloc`'s `unsafe` is statically prevented.
    ///
    /// # Panics
    /// Panics on out-of-memory, matching [`PBox::new_in`](crate::PBox::new_in).
    #[track_caller]
    #[inline]
    pub fn alloc_in(memcx: &MemCx<'mcx>) -> Self {
        let bytes = memcx
            .alloc_bytes(core::mem::size_of::<T>())
            .expect("PgBoxIn::alloc_in: out of memory");
        PgBoxIn {
            ptr: Some(bytes.cast::<T>()),
            _cx: PhantomData,
            _alloc: PhantomData,
        }
    }

    /// Allocate enough memory for a `T` inside `memcx`. The memory is
    /// zero-filled.
    ///
    /// See [`alloc_in`](Self::alloc_in) for the safety contract.
    ///
    /// # Panics
    /// Panics on out-of-memory.
    #[track_caller]
    #[inline]
    pub fn alloc0_in(memcx: &MemCx<'mcx>) -> Self {
        let bytes = memcx
            .alloc_zeroed_bytes(core::mem::size_of::<T>())
            .expect("PgBoxIn::alloc0_in: out of memory");
        PgBoxIn {
            ptr: Some(bytes.cast::<T>()),
            _cx: PhantomData,
            _alloc: PhantomData,
        }
    }
}
```

- [ ] **Step 4: Verify compile passes**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx --no-run
```

Expected: clean compile.

- [ ] **Step 5: Commit**

```bash
git add pgrx/src/pgbox/in_mcx.rs
git commit -m "feat(pgbox): safe alloc_in / alloc0_in via MemCx allocation API"
```

---

## Task 6: Implement `Deref` and `DerefMut`

**Files:**
- Modify: `pgrx/src/pgbox/in_mcx.rs`

- [ ] **Step 1: Write the failing test**

Inside the `tests` module:

```rust
    #[test]
    fn deref_reads_pointee() {
        let mut value: i32 = 123;
        let ptr = &mut value as *mut i32;
        let memcx = unsafe { crate::memcx::MemCx::from_ptr(core::ptr::dangling_mut()) };
        let b: PgBoxIn<'_, i32, AllocatedByPostgres> =
            unsafe { PgBoxIn::from_pg_in(ptr, &memcx) };
        assert_eq!(*b, 123);
    }

    #[test]
    fn deref_mut_writes_pointee() {
        let mut value: i32 = 0;
        let ptr = &mut value as *mut i32;
        let memcx = unsafe { crate::memcx::MemCx::from_ptr(core::ptr::dangling_mut()) };
        let mut b: PgBoxIn<'_, i32, AllocatedByPostgres> =
            unsafe { PgBoxIn::from_pg_in(ptr, &memcx) };
        *b = 456;
        drop(b);
        assert_eq!(value, 456);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx::tests::deref_reads_pointee
```

Expected: FAIL with `type 'PgBoxIn<...>' cannot be dereferenced`.

- [ ] **Step 3: Implement `Deref` and `DerefMut`**

Append to `pgrx/src/pgbox/in_mcx.rs` (above the `tests` module):

```rust
use core::ops::{Deref, DerefMut};

impl<'mcx, T, A: WhoAllocated> Deref for PgBoxIn<'mcx, T, A> {
    type Target = T;

    #[track_caller]
    fn deref(&self) -> &Self::Target {
        match self.ptr.as_ref() {
            Some(p) => unsafe { p.as_ref() },
            None => panic!("Attempt to dereference null pointer during Deref of PgBoxIn"),
        }
    }
}

impl<'mcx, T, A: WhoAllocated> DerefMut for PgBoxIn<'mcx, T, A> {
    #[track_caller]
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self.ptr.as_mut() {
            Some(p) => unsafe { p.as_mut() },
            None => panic!("Attempt to dereference null pointer during DerefMut of PgBoxIn"),
        }
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx::tests
```

Expected: all tests PASS (now 7).

- [ ] **Step 5: Commit**

```bash
git add pgrx/src/pgbox/in_mcx.rs
git commit -m "feat(pgbox): Deref/DerefMut for PgBoxIn (panic on null, matches PgBox)"
```

---

## Task 7: Implement `into_pg` and `into_pg_boxed`

**Files:**
- Modify: `pgrx/src/pgbox/in_mcx.rs`

- [ ] **Step 1: Write the failing tests**

Inside the `tests` module:

```rust
    #[test]
    fn into_pg_returns_same_pointer_and_suppresses_drop() {
        FAKE_PFREE_CALLS.store(0, Ordering::SeqCst);
        let mut value: i32 = 7;
        let ptr = &mut value as *mut i32;
        let b: PgBoxIn<'_, i32, AllocatedByCounter> = PgBoxIn {
            ptr: NonNull::new(ptr),
            _cx: PhantomData,
            _alloc: PhantomData,
        };
        let returned = b.into_pg();
        assert_eq!(returned, ptr);
        // Drop was suppressed by `into_pg` consuming `self` and forgetting it.
        assert_eq!(FAKE_PFREE_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn into_pg_boxed_yields_legacy_pgbox_with_same_ptr() {
        let mut value: i32 = 11;
        let ptr = &mut value as *mut i32;
        let memcx = unsafe { crate::memcx::MemCx::from_ptr(core::ptr::dangling_mut()) };
        let b: PgBoxIn<'_, i32, AllocatedByPostgres> =
            unsafe { PgBoxIn::from_pg_in(ptr, &memcx) };
        let legacy: super::PgBox<i32, AllocatedByPostgres> = b.into_pg_boxed();
        assert_eq!(legacy.as_ptr(), ptr);
    }

    #[test]
    fn into_pg_on_null_returns_null() {
        let b: PgBoxIn<'_, i32, AllocatedByPostgres> = PgBoxIn::null();
        assert!(b.into_pg().is_null());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx::tests::into_pg_returns_same_pointer_and_suppresses_drop
```

Expected: FAIL with `no method named 'into_pg'`.

- [ ] **Step 3: Implement `into_pg` / `into_pg_boxed`**

Add to the `impl<'mcx, T, A: WhoAllocated> PgBoxIn<'mcx, T, A>` block (the one that already contains `null` / `is_null` / `as_ptr`):

```rust
    /// Hand the wrapped pointer back to Postgres, dropping the `'mcx` brand.
    ///
    /// The `PgBoxIn`'s `Drop` is suppressed — Postgres now owns the lifetime.
    #[inline]
    pub fn into_pg(self) -> *mut T {
        let raw = self.as_ptr();
        // Suppress Drop without using `mem::forget` on a generic — explicit field
        // dance is not needed; ManuallyDrop preserves clarity.
        let _ = core::mem::ManuallyDrop::new(self);
        raw
    }

    /// Convert to a legacy un-lifetimed [`PgBox<T, AllocatedByPostgres>`](super::PgBox)
    /// for hand-off into existing pgrx APIs that have not yet adopted the lifetime-bound
    /// shape.
    #[inline]
    pub fn into_pg_boxed(self) -> super::PgBox<T, AllocatedByPostgres> {
        // SAFETY: `into_pg` returns a pointer that was either Postgres-owned to begin
        // with, or has just been transferred to Postgres' ownership. Either way it is
        // a valid input to `PgBox::from_pg`.
        unsafe { super::PgBox::from_pg(self.into_pg()) }
    }
```

- [ ] **Step 4: Run tests**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx::tests
```

Expected: all tests PASS (now 10).

- [ ] **Step 5: Commit**

```bash
git add pgrx/src/pgbox/in_mcx.rs
git commit -m "feat(pgbox): into_pg / into_pg_boxed hand ownership back to Postgres"
```

---

## Task 8: Add the soundness `compile_fail` doctest

**Files:**
- Modify: `pgrx/src/pgbox/in_mcx.rs` (the `PgBoxIn` doc comment)

This is the headline soundness proof for the whole feature.

- [ ] **Step 1: Add the doctest to the `PgBoxIn` docstring**

Replace the existing `///` block above `pub struct PgBoxIn` with:

```rust
/// Similar to [`PgBox`](super::PgBox), but the wrapped pointer's lifetime is
/// tied to a [`MemCx<'mcx>`].
///
/// The borrow checker enforces that the box cannot outlive the memory context
/// the pointer was allocated in — closing the cross-context use-after-free
/// hole that `PgBox` has by design (see upstream issue
/// [pgcentralfoundation/pgrx#2204](https://github.com/pgcentralfoundation/pgrx/issues/2204)).
///
/// ## Example: lifetime escape is rejected at compile time
///
/// ```compile_fail,E0597
/// use pgrx::memcx;
/// use pgrx::pgbox::PgBoxIn;
/// use pgrx::pgbox::AllocatedByPostgres;
///
/// let escaped = memcx::current_context(|cx| {
///     // `cx` only lives for this closure, so the returned `PgBoxIn` cannot
///     // outlive it. The compiler rejects the attempt to return it.
///     unsafe {
///         PgBoxIn::<i32, AllocatedByPostgres>::from_pg_in(
///             std::ptr::null_mut(),
///             cx,
///         )
///     }
/// });
/// let _ = escaped.is_null();
/// ```
```

- [ ] **Step 2: Run the doctest**

```bash
cargo +nightly test -p pgrx --features pg17 --doc pgbox::in_mcx::PgBoxIn
```

Expected: PASS — the doctest passes when the wrapped code **fails to compile** (because of `compile_fail`).

If the doctest *does* compile (which would mean a soundness failure), this PASS turns into FAIL. That is intentional: the doctest is the regression test for the brand.

- [ ] **Step 3: Commit**

```bash
git add pgrx/src/pgbox/in_mcx.rs
git commit -m "test(pgbox): compile_fail doctest proves PgBoxIn rejects lifetime escape"
```

---

## Task 9: Scaffold the `pgbox_in_demo` example crate

**Files:**
- Create: `pgrx-examples/pgbox_in_demo/Cargo.toml`
- Create: `pgrx-examples/pgbox_in_demo/pgbox_in_demo.control`
- Create: `pgrx-examples/pgbox_in_demo/src/lib.rs` (skeleton only — `#[pg_test]` integration in Task 10)
- Create: `pgrx-examples/pgbox_in_demo/README.md`

- [ ] **Step 1: Create `Cargo.toml`**

```toml
#LICENSE Portions Copyright 2019-2021 ZomboDB, LLC.
#LICENSE
#LICENSE Portions Copyright 2021-2023 Technology Concepts & Design, Inc.
#LICENSE
#LICENSE Portions Copyright 2023-2023 PgCentral Foundation, Inc. <contact@pgcentral.org>
#LICENSE
#LICENSE All rights reserved.
#LICENSE
#LICENSE Use of this source code is governed by the MIT license that can be found in the LICENSE file.

[package]
name = "pgbox_in_demo"
version = "0.0.0"
edition.workspace = true
rust-version.workspace = true
publish = false

[lib]
crate-type = ["cdylib"]

[features]
default = ["pg17"]
pg13 = ["pgrx/pg13", "pgrx-tests/pg13"]
pg14 = ["pgrx/pg14", "pgrx-tests/pg14"]
pg15 = ["pgrx/pg15", "pgrx-tests/pg15"]
pg16 = ["pgrx/pg16", "pgrx-tests/pg16"]
pg17 = ["pgrx/pg17", "pgrx-tests/pg17"]
pg18 = ["pgrx/pg18", "pgrx-tests/pg18"]
pg_test = []

[dependencies]
pgrx = { path = "../../pgrx" }

[dev-dependencies]
pgrx-tests = { path = "../../pgrx-tests" }
```

- [ ] **Step 2: Create the `.control` file**

```text
comment = 'PgBoxIn lifetime-bound box demo'
default_version = '@CARGO_VERSION@'
module_pathname = '$libdir/pgbox_in_demo'
relocatable = true
superuser = false
schema = 'pgbox_in_demo'
```

- [ ] **Step 3: Create `src/lib.rs` skeleton**

```rust
//LICENSE Portions Copyright 2019-2021 ZomboDB, LLC.
//LICENSE
//LICENSE Portions Copyright 2021-2023 Technology Concepts & Design, Inc.
//LICENSE
//LICENSE Portions Copyright 2023-2023 PgCentral Foundation, Inc. <contact@pgcentral.org>
//LICENSE
//LICENSE All rights reserved.
//LICENSE
//LICENSE Use of this source code is governed by the MIT license that can be found in the LICENSE file.

//! Demonstration of [`pgrx::pgbox::PgBoxIn`] — the lifetime-bound sibling of
//! `PgBox`. See `docs/superpowers/specs/2026-06-14-pgbox-plan-b-design.md`.

use pgrx::prelude::*;
use pgrx::memcx;
use pgrx::pgbox::PgBoxIn;

pgrx::pg_module_magic!(name, version);

/// Allocates an `ItemPointerData` inside the current memory context and
/// returns it to Postgres. The lifetime brand on `PgBoxIn` ensures we cannot
/// accidentally leak the box out of the closure.
#[pg_extern]
fn pgbox_in_alloc_demo() -> pg_sys::ItemPointer {
    memcx::current_context(|cx| {
        let mut tid: PgBoxIn<'_, pg_sys::ItemPointerData, _> = PgBoxIn::alloc0_in(cx);
        tid.ip_posid = 42;
        // Hand off to Postgres' ownership; the `'mcx` brand is dropped here.
        tid.into_pg_boxed().into_pg()
    })
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    // Integration tests are added in Task 10.
}

#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}
    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec![]
    }
}
```

- [ ] **Step 4: Create `README.md`**

```markdown
# pgbox_in_demo

Demonstrates `pgrx::pgbox::PgBoxIn<'mcx, T, A>` — the lifetime-bound sibling
of `PgBox<T, A>` introduced for [issue #2204](https://github.com/pgcentralfoundation/pgrx/issues/2204).

## Why `PgBoxIn` over `PgBox`?

`PgBox::alloc()` is `unsafe` because it cannot prove that the wrapped pointer
will not outlive the `MemoryContext` it was allocated in. `PgBoxIn::alloc_in`
is **safe** because the returned box carries the `'mcx` lifetime of the
`MemCx` it was allocated from — the borrow checker rejects any attempt to
escape the context.

```rust
// PgBox — unsafe, dangling possible
let tid = unsafe { PgBox::<pg_sys::ItemPointerData>::alloc0() };

// PgBoxIn — safe, lifetime-bound
let returned = memcx::current_context(|cx| {
    let mut tid = PgBoxIn::<pg_sys::ItemPointerData, _>::alloc0_in(cx);
    tid.ip_posid = 42;
    tid.into_pg_boxed().into_pg()
});
```

## Running

```bash
cargo pgrx run pg17 --package pgbox_in_demo
```

Then inside `psql`:

```sql
CREATE EXTENSION pgbox_in_demo;
SELECT pgbox_in_alloc_demo();
```

## Testing

```bash
cargo pgrx test pg17 --package pgbox_in_demo
```
```

- [ ] **Step 5: Verify the workspace picks it up**

```bash
cargo +nightly check -p pgbox_in_demo --features pg17
```

Expected: clean compile.

- [ ] **Step 6: Commit**

```bash
git add pgrx-examples/pgbox_in_demo/
git commit -m "feat(examples): add pgbox_in_demo showcasing PgBoxIn"
```

---

## Task 10: Add `#[pg_test]` integration tests inside the example

**Files:**
- Modify: `pgrx-examples/pgbox_in_demo/src/lib.rs`

These tests run inside an actual Postgres backend, so they exercise the real `palloc`/`pfree` paths.

- [ ] **Step 1: Replace the empty `tests` module with real tests**

Replace the `#[cfg(any(test, feature = "pg_test"))] #[pg_schema] mod tests { ... }` block with:

```rust
#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::memcx;
    use pgrx::pgbox::{AllocatedByRust, PgBoxIn};
    use pgrx::prelude::*;

    /// `alloc_in` returns a pointer we can write to and read back.
    #[pg_test]
    fn alloc_in_writable() {
        memcx::current_context(|cx| {
            let mut b: PgBoxIn<'_, i64, AllocatedByRust> = PgBoxIn::alloc_in(cx);
            *b = 0xCAFEBABEi64;
            assert_eq!(*b, 0xCAFEBABE);
            // Drop here calls pfree — must not crash.
        });
    }

    /// `alloc0_in` zero-fills.
    #[pg_test]
    fn alloc0_in_is_zero_filled() {
        memcx::current_context(|cx| {
            let b: PgBoxIn<'_, [u8; 64], AllocatedByRust> = PgBoxIn::alloc0_in(cx);
            for byte in b.iter() {
                assert_eq!(*byte, 0);
            }
        });
    }

    /// `into_pg` suppresses Drop — no double-free when we then manually `pfree`.
    #[pg_test]
    fn into_pg_then_manual_pfree() {
        let raw = memcx::current_context(|cx| {
            let b: PgBoxIn<'_, i32, AllocatedByRust> = PgBoxIn::alloc0_in(cx);
            b.into_pg()
        });
        // Manually free — if Drop had also freed, this would crash or trip assertions.
        unsafe { pg_sys::pfree(raw.cast()); }
    }

    /// `from_pg_in` followed by Drop must NOT free (Postgres owns the pointer).
    #[pg_test]
    fn from_pg_in_drop_does_not_free() {
        // Allocate via raw palloc so we own the pointer for cleanup.
        let raw: *mut i32 = unsafe { pg_sys::palloc0(core::mem::size_of::<i32>()).cast() };
        memcx::current_context(|cx| {
            let b: PgBoxIn<'_, i32, _> = unsafe { PgBoxIn::from_pg_in(raw, cx) };
            assert!(!b.is_null());
            // Drop here is a no-op for AllocatedByPostgres.
        });
        // Pointer must still be live and freeable.
        unsafe { pg_sys::pfree(raw.cast()); }
    }

    /// End-to-end: the public `pgbox_in_alloc_demo()` SQL function works.
    #[pg_test]
    fn alloc_demo_sql() {
        let result: Option<pg_sys::ItemPointer> =
            Spi::get_one("SELECT pgbox_in_alloc_demo()").expect("SPI");
        assert!(result.is_some());
    }
}
```

- [ ] **Step 2: Run the integration tests**

```bash
cargo pgrx test pg17 --package pgbox_in_demo
```

Expected: all 5 tests PASS, no crashes, no leaks reported.

- [ ] **Step 3: Commit**

```bash
git add pgrx-examples/pgbox_in_demo/src/lib.rs
git commit -m "test(pgbox_in_demo): #[pg_test] integration coverage for PgBoxIn"
```

---

## Task 11: Final verification

- [ ] **Step 1: Full workspace type-check**

```bash
cd /home/azureuser/pgrx
cargo +nightly check --workspace --all-features --all-targets
```

Expected: clean. Address any new warnings introduced by the new module.

- [ ] **Step 2: Full unit test pass for pgrx**

```bash
cargo +nightly test -p pgrx --features pg17 --lib pgbox::in_mcx
```

Expected: all 10 unit tests + 1 doctest PASS.

- [ ] **Step 3: Integration test pass**

```bash
cargo pgrx test pg17 --package pgbox_in_demo
```

Expected: 5 PASS.

- [ ] **Step 4: Verify nothing else regressed**

```bash
cargo pgrx test pg17 --package strings
```

Expected: existing strings example still passes — proves we did not break the legacy `PgBox` path.

- [ ] **Step 5: Tag the implementation as ready**

```bash
git log --oneline -12
```

Verify the commit chain is clean and each commit is reviewable in isolation.

---

## Open Questions for Maintainers (carry into PR description)

These mirror the spec §7. Surface them in the PR description, not as inline `// TODO`s in code:

1. **Naming**: `PgBoxIn` (mirrors `Box::new_in`) vs `ScopedPgBox` / `BoundPgBox`. Open to bikeshed.
2. **Two-stage roadmap**: is the team open to a follow-up that consolidates `'mcx` into `PgBox` itself with `'mcx = 'static` default once `PgBoxIn` proves itself?
3. **`From` direction**: should we additionally provide `From<PgBox<T, AllocatedByPostgres>> for PgBoxIn<'static, T, AllocatedByPostgres>` to ease incremental opt-in? Deferred from S1.
4. **Module placement**: keep at `pgrx::pgbox::PgBoxIn`, or move both to `pgrx::palloc::` next to `PBox`?

---

## Self-Review Notes

**Spec coverage:** Tasks 2-7 cover §3.3 API surface. Task 8 is §5.2 compile-fail doctest. Tasks 9-10 cover §5.3 integration + §5.4 example. Task 1 is §3.1 file layout. Task 11 is §6 verification checklist. ✓

**Type consistency:** `PgBoxIn<'mcx, T, AllocatedBy: WhoAllocated = AllocatedByPostgres>` is the same shape across all tasks. `_cx`, `_alloc`, `ptr` field names match across constructions. `into_pg_boxed()` returns `super::PgBox<T, AllocatedByPostgres>` consistently. ✓

**Placeholder scan:** Task 5 carries an explicit deferral note (runtime testing in Task 10) rather than "TODO". Open questions are §"Open Questions for Maintainers" not inline code. ✓

**Risk callouts that surfaced during review:**
- `MemCx::from_ptr` is `pub(crate)` — fine because tests live inside the `pgrx` crate.
- `WhoAllocated::maybe_pfree` may or may not be on an `unsafe trait`; Task 3 notes the verification step.
- `core::ptr::dangling_mut()` requires Rust 1.84+; MSRV 1.96 covers it.
- The example crate's `default = ["pg17"]` differs from `strings`'s `default = ["pg13"]`. This is intentional (pg17 is the test target everywhere else in this plan); reviewers may push back, in which case switch to `pg13` for symmetry — no code change needed.
