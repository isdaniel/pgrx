# pgbox_in_demo

Demonstrates `pgrx::pgbox::PgBoxIn<'mcx, T, A>` — the lifetime-bound sibling
of `PgBox<T, A>` introduced for [issue #2204](https://github.com/pgcentralfoundation/pgrx/issues/2204).

## Why `PgBoxIn` over `PgBox`?

`PgBox::alloc()` is `unsafe` because it cannot prove that the wrapped pointer
will not outlive the `MemoryContext` it was allocated in. `PgBoxIn::alloc_in`
is **safe** because the returned box carries the `'mcx` lifetime of the
`MemCx` it was allocated from — the borrow checker rejects any attempt for a
borrow into the box to escape the context's scope.

```rust
// PgBox — unsafe, dangling possible
let tid = unsafe { PgBox::<pg_sys::ItemPointerData>::alloc0() };

// PgBoxIn — safe, lifetime-bound
let posid = memcx::current_context(|cx| {
    let mut tid = PgBoxIn::<pg_sys::ItemPointerData, _>::alloc0_in(cx);
    tid.ip_posid = 42;
    tid.into_pg_boxed().ip_posid as i32
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
