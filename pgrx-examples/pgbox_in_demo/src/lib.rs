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

/// Allocates an `ItemPointerData` inside the current memory context, writes a
/// known value into it, and returns the recovered field. The lifetime brand
/// on `PgBoxIn` ensures that references into the closure-local box cannot
/// leak out of the `memcx::current_context` scope.
#[pg_extern]
fn pgbox_in_alloc_demo() -> i32 {
    memcx::current_context(|cx| {
        let mut tid: PgBoxIn<'_, pg_sys::ItemPointerData, _> = PgBoxIn::alloc0_in(cx);
        tid.ip_posid = 42;
        // Hand off to Postgres' ownership; the `'mcx` brand is dropped here.
        let owned = tid.into_pg_boxed();
        owned.ip_posid as i32
    })
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::memcx;
    use pgrx::pgbox::{AllocatedByRust, PgBoxIn};
    use pgrx::prelude::*;

    /// `alloc_in` returns a pointer we can write to and read back, and Drop
    /// (which calls `pfree`) does not crash.
    #[pg_test]
    fn alloc_in_writable() {
        memcx::current_context(|cx| {
            let mut b: PgBoxIn<'_, i64, AllocatedByRust> = PgBoxIn::alloc_in(cx);
            *b = 0xCAFEBABEi64;
            assert_eq!(*b, 0xCAFEBABE);
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

    /// End-to-end: the public `pgbox_in_alloc_demo()` SQL function works and
    /// returns the value we wrote (42) inside the extension.
    #[pg_test]
    fn alloc_demo_sql_returns_42() {
        let result: Option<i32> =
            Spi::get_one("SELECT pgbox_in_demo.pgbox_in_alloc_demo()").expect("SPI");
        assert_eq!(result, Some(42));
    }
}

#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}
    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec![]
    }
}
