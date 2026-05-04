# Predicates

No predicate means an unconditional persisted trap.

```js
gb.add_breakpoint(0x0150);
```

A string predicate is persisted:

```js
gb.add_breakpoint(0x0150, "regs.a == 0x42");
```

String predicates run with a narrow debugger scope:

- `regs`: current register snapshot.
- `pc`: current program counter.
- `access`: memory access record for watchpoints.
- `cycle`: current emulator cycle.
- `symbol(name)` and `symbolInBank(name, bank)`: symbol lookup helpers.

A closure predicate is only for the current `exec` invocation and should be treated as transient:

```js
gb.add_breakpoint(0x0150, () => gb.regs.a == 0x42);
```

The session will not serialize closure environments. Expect a `predicate_not_persisted` warning.
