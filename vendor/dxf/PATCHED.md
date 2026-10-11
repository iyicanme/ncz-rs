# Patched copy of dxf 0.6.2

This is [dxf 0.6.2](https://crates.io/crates/dxf/0.6.2) from crates.io, used through `[patch.crates-io]` in the
top-level `Cargo.toml`. Changes are marked with `ncz-rs patch` comments:

- `src/entity.rs`, `Entity::add_code_pairs`: extended data (XDATA) is written before the vertices and `SEQEND` of a
  `POLYLINE` and the attributes of an `INSERT`, not after them. Written after them, readers such as ezdxf attach it
  to the `SEQEND`.
- `src/drawing.rs`, `ensure_block_entity_handles_are_set`: entities added to a block get handles for their vertices,
  attributes and `SEQEND` too, as `add_entity` gives entities of the model space. They were written as handle `0`,
  and ezdxf could not open such files. The shared code is `set_child_handles`.
- `src/block.rs`: a block has a separate `ENDBLK` handle (`__end_handle`), read from the file and assigned when the
  block is added. The `ENDBLK` used to repeat the block's handle. The test `write_block_r12_compat` expects the new
  handles.
- `src/block.rs`: the test `write_block_polyline_with_handles_and_x_data` covers the three changes above. Each of its
  assertions fails on the unpatched release.
- `build/table_generator.rs`, `src/drawing.rs`: tables have handles (group 5) and owner `0` (group 330), and their
  records are owned by them. Upstream left this as a TODO, and readers such as ezdxf repaired every table head.
  Saving does not change the drawing, so the tables take the handles from the handle seed on (`TABLE_COUNT`, one per
  table) and `$HANDSEED` is written past them. The tests `write_proper_handseed_on_new_file` and
  `write_proper_handseed_on_read_file` expect the moved seed, and `write_unique_handles_including_tables` checks that
  every handle is set, unique and below the seed. It fails on the unpatched release.

The tests `as_datetime_conversion_test` and `as_double_conversion_test` fail on the unpatched release too.

Remove this copy and the patch entry once a release fixes these upstream (https://github.com/ixmilia/dxf-rs).
