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

use crate::memcx::MemCx;
use crate::pgbox::{AllocatedByPostgres, WhoAllocated};
use core::marker::PhantomData;
use core::ptr::NonNull;

/// Similar to [`PgBox`](super::PgBox), but the wrapped pointer's lifetime is
/// tied to a [`MemCx<'mcx>`].
///
/// The borrow checker prevents a `PgBoxIn` from being constructed against a
/// `MemCx` it cannot prove outlives. Combined with brand-introducing APIs
/// (e.g. a future `with_owned_context`) this closes the cross-context
/// use-after-free hole that `PgBox` has by design (see upstream issue
/// [pgcentralfoundation/pgrx#2204](https://github.com/pgcentralfoundation/pgrx/issues/2204)).
///
/// **Caveat for [`memcx::current_context`]:** that entry point's `'curr`
/// lifetime is caller-inferred, so a `PgBoxIn<'curr, T>` *value* can still
/// be assigned a wider lifetime by the caller. The brand currently bites
/// for borrows that escape the closure (see the doctests below) but not for
/// values returned through lifetime inference. Tightening this requires a
/// brand-introducing wrapper, tracked as a follow-up.
///
/// ## Example: lifetime escape is rejected at compile time
///
/// The closure-bound `&MemCx` cannot itself escape `current_context`, and
/// neither can a borrow of any `PgBoxIn` constructed from it. This mirrors
/// the borrow rejection that protects `SpiClient`.
///
/// ```compile_fail
/// use pgrx::memcx;
///
/// // Attempting to return the borrowed `MemCx` from the closure fails:
/// // the inner closure lifetime cannot escape into the outer scope.
/// let escaped = memcx::current_context(|cx| cx);
/// let _ = escaped;
/// ```
///
/// ```compile_fail
/// use pgrx::memcx;
/// use pgrx::pgbox::{PgBoxIn, AllocatedByRust};
///
/// // A reference into a `PgBoxIn` that lives only on the closure's stack
/// // cannot escape: the closure-local borrow is rejected.
/// let escaped_ref = memcx::current_context(|cx| {
///     let b: PgBoxIn<'_, i32, AllocatedByRust> = PgBoxIn::alloc_in(cx);
///     &b
/// });
/// let _ = escaped_ref;
/// ```
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

    /// Hand the wrapped pointer back to Postgres, dropping the `'mcx` brand.
    ///
    /// The `PgBoxIn`'s `Drop` is suppressed — Postgres now owns the lifetime.
    #[inline]
    pub fn into_pg(self) -> *mut T {
        let raw = self.as_ptr();
        // Suppress Drop without using `mem::forget` on a generic.
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
}

impl<'mcx, T, A: WhoAllocated> Drop for PgBoxIn<'mcx, T, A> {
    fn drop(&mut self) {
        if let Some(ptr) = self.ptr {
            // SAFETY: `WhoAllocated::maybe_pfree` is the trait's contract: each
            // implementor decides whether the pointer should be freed. The pointer
            // was either supplied by the caller (asserted valid via the `unsafe`
            // constructors) or produced by our own `alloc_in` family.
            unsafe { A::maybe_pfree(ptr.as_ptr().cast()); }
        }
    }
}

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

unsafe impl<'mcx, T, A> pgrx_sql_entity_graph::metadata::SqlTranslatable
    for PgBoxIn<'mcx, T, A>
where
    T: pgrx_sql_entity_graph::metadata::SqlTranslatable,
    A: WhoAllocated,
{
    const TYPE_IDENT: &'static str = T::TYPE_IDENT;
    const TYPE_ORIGIN: pgrx_sql_entity_graph::metadata::TypeOrigin = T::TYPE_ORIGIN;
    const ARGUMENT_SQL: Result<
        pgrx_sql_entity_graph::metadata::SqlMappingRef,
        pgrx_sql_entity_graph::metadata::ArgumentError,
    > = T::ARGUMENT_SQL;
    const RETURN_SQL: Result<
        pgrx_sql_entity_graph::metadata::ReturnsRef,
        pgrx_sql_entity_graph::metadata::ReturnsError,
    > = T::RETURN_SQL;
}

unsafe impl<'mcx, T> crate::callconv::BoxRet for PgBoxIn<'mcx, T, AllocatedByRust> {
    unsafe fn box_into<'fcx>(
        self,
        fcinfo: &mut crate::callconv::FcInfo<'fcx>,
    ) -> crate::datum::Datum<'fcx> {
        let ptr_opt = self.ptr;
        // Suppress Drop — Postgres takes ownership; pfreeing here would
        // double-free when the context resets.
        let _ = core::mem::ManuallyDrop::new(self);

        match ptr_opt {
            Some(ptr) => unsafe {
                fcinfo.return_raw_datum(crate::pg_sys::Datum::from(ptr.as_ptr() as *mut u8))
            },
            None => fcinfo.return_null(),
        }
    }
}

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

    use core::sync::atomic::{AtomicUsize, Ordering};

    static FAKE_PFREE_CALLS: AtomicUsize = AtomicUsize::new(0);

    struct AllocatedByCounter;
    impl WhoAllocated for AllocatedByCounter {
        unsafe fn maybe_pfree(_ptr: *mut std::os::raw::c_void) {
            FAKE_PFREE_CALLS.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn drop_honors_who_allocated() {
        FAKE_PFREE_CALLS.store(0, Ordering::SeqCst);

        // Null PgBoxIn: Drop must NOT call maybe_pfree.
        {
            let _b: PgBoxIn<'_, i32, AllocatedByCounter> = PgBoxIn::null();
        }
        assert_eq!(FAKE_PFREE_CALLS.load(Ordering::SeqCst), 0,
            "null PgBoxIn must not call maybe_pfree on Drop");

        // Non-null PgBoxIn: Drop MUST call maybe_pfree exactly once.
        // SAFETY: AllocatedByCounter::maybe_pfree only increments a counter; it
        // never dereferences or frees the pointer, so a stack pointer is safe here.
        // Do NOT copy this pattern with AllocatedByRust — that would pfree a
        // stack pointer and trigger UB.
        let mut value: i32 = 7;
        let ptr = &mut value as *mut i32;
        {
            let _b: PgBoxIn<'_, i32, AllocatedByCounter> = PgBoxIn {
                ptr: NonNull::new(ptr),
                _cx: PhantomData,
                _alloc: PhantomData,
            };
            assert_eq!(FAKE_PFREE_CALLS.load(Ordering::SeqCst), 0,
                "maybe_pfree must not run before Drop");
        }
        assert_eq!(FAKE_PFREE_CALLS.load(Ordering::SeqCst), 1,
            "non-null PgBoxIn must call maybe_pfree exactly once on Drop");

        // Now verify into_pg suppresses Drop on the same counter.
        FAKE_PFREE_CALLS.store(0, Ordering::SeqCst);
        let mut value2: i32 = 8;
        let ptr2 = &mut value2 as *mut i32;
        let b: PgBoxIn<'_, i32, AllocatedByCounter> = PgBoxIn {
            ptr: NonNull::new(ptr2),
            _cx: PhantomData,
            _alloc: PhantomData,
        };
        let returned = b.into_pg();
        assert_eq!(returned, ptr2);
        assert_eq!(FAKE_PFREE_CALLS.load(Ordering::SeqCst), 0,
            "into_pg must suppress Drop");
    }

    #[test]
    fn into_pg_boxed_yields_legacy_pgbox_with_same_ptr() {
        let mut value: i32 = 11;
        let ptr = &mut value as *mut i32;
        let memcx = unsafe { crate::memcx::MemCx::from_ptr(core::ptr::dangling_mut()) };
        let b: PgBoxIn<'_, i32, AllocatedByPostgres> =
            unsafe { PgBoxIn::from_pg_in(ptr, &memcx) };
        let legacy: crate::pgbox::PgBox<i32, AllocatedByPostgres> = b.into_pg_boxed();
        assert_eq!(legacy.as_ptr(), ptr);
    }

    #[test]
    fn into_pg_on_null_returns_null() {
        let b: PgBoxIn<'_, i32, AllocatedByPostgres> = PgBoxIn::null();
        assert!(b.into_pg().is_null());
    }

    #[test]
    fn from_pg_in_round_trips_pointer() {
        let mut value: i32 = 99;
        let ptr = &mut value as *mut i32;
        // SAFETY: the MemCx stub is never dereferenced — from_pg_in only requires a
        // borrow with the right lifetime; we never invoke any MemCx method.
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

    /// Compile-time-only check: `alloc_in` and `alloc0_in` must be `safe` (no
    /// `unsafe` keyword) and must produce a `PgBoxIn` that carries the input
    /// `MemCx`'s lifetime. Runtime behavior is exercised in pgrx-examples/pgbox_in_demo.
    #[allow(dead_code)]
    fn _compile_check_alloc_in_signatures<'mcx>(cx: &crate::memcx::MemCx<'mcx>) {
        let _: PgBoxIn<'mcx, i32, crate::pgbox::AllocatedByRust> = PgBoxIn::alloc_in(cx);
        let _: PgBoxIn<'mcx, i32, crate::pgbox::AllocatedByRust> = PgBoxIn::alloc0_in(cx);
    }

    /// Compile-time check: `PgBoxIn` implements `SqlTranslatable` so it can
    /// appear in `#[pg_extern]` signatures. Runtime SQL behavior is exercised
    /// in pgrx-examples/pgbox_in_demo (Task 3 of the R1 plan).
    #[allow(dead_code)]
    fn _compile_check_sql_translatable<'mcx>() {
        fn _requires_sql_translatable<T: pgrx_sql_entity_graph::metadata::SqlTranslatable>() {}
        _requires_sql_translatable::<PgBoxIn<'mcx, i32, AllocatedByPostgres>>();
        _requires_sql_translatable::<PgBoxIn<'mcx, i32, AllocatedByRust>>();
    }

    /// Compile-time check: `PgBoxIn<_, _, AllocatedByRust>` implements `BoxRet`,
    /// the trait macros need for `#[pg_extern]` return types. Runtime exercise
    /// in pgrx-examples/pgbox_in_demo (Task 3 of the R1 plan).
    #[allow(dead_code)]
    fn _compile_check_box_ret<'mcx>() {
        fn _requires_box_ret<T: crate::callconv::BoxRet>() {}
        _requires_box_ret::<PgBoxIn<'mcx, i32, AllocatedByRust>>();
        _requires_box_ret::<PgBoxIn<'mcx, crate::pg_sys::ItemPointerData, AllocatedByRust>>();
        // Deliberately NOT requiring BoxRet for AllocatedByPostgres —
        // semantics intentionally restricted (see spec §3.3).
    }
}
