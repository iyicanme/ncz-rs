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
- `build/table_generator.rs`: `DIMSTYLE` records write their handle in group 105, as the DXF reference requires, and
  are read from 105 as well as 5. With group 5, AutoCAD stopped loading at the first `DIMSTYLE` record. The
  `DIMSTYLE` table also writes its subclass marker `AcDbDimStyleTable` and group 71 from R13 on.

- `src/stream_writer.rs` (new): `Drawing::save_streaming` writes a drawing up to its entities and returns an
  `EntityStreamWriter`, which writes further entities one at a time, giving them handles from a range set aside in
  `$HANDSEED`, so a large drawing need not hold its entities or collect its code pairs. `Drawing::prepare_entity` adds
  the table entries an entity refers to, as `add_entity` does, through `ensure_entity_tables` in `src/drawing.rs`,
  and `handles_needed` counts the handles to set aside. The tests `streamed_entities_match_saved_ones` and
  `stops_when_the_reserved_handles_run_out` cover it.
- `src/code_pair_writer.rs`, `src/drawing.rs`, `src/stream_writer.rs`: `Drawing::text_encoding` writes the text of
  versions up to R2004 in a code page, escaping as `\U+` only the characters it cannot encode. Upstream escapes every
  non-ASCII character. It is `None` by default, which keeps upstream's behaviour. The test `encodes_with_escapes` covers
  it.
- `src/entity.rs`, `Entity::add_code_pairs`: the vertices and `SEQEND` of a `POLYLINE`, and the `SEQEND` of an `INSERT`
  with attributes, are written with the layer and space of their entity, and it owns them (group 330). They were
  written on layer `0` without an owner, and the `SEQEND` without its common values. The tests `write_2d_polyline` and
  `write_3d_polyline` expect the owner, and `polyline_children_take_its_layer` checks the layer. It fails on the
  unpatched release.
- `spec/TableSpec.xml`: layers have a true color, `Layer::color_24_bit` (group 420, R2004 on), as entities do.
- `spec/EntitiesSpec.xml`: an entity's `transparency` (group 440) is left out when it is 0, which is not a valid
  value. ezdxf repaired it on every entity of an R2004 file. The test `write_layer_true_color_and_no_default_transparency`
  covers both.

The tests `as_datetime_conversion_test` and `as_double_conversion_test` fail on the unpatched release too.

Remove this copy and the patch entry once a release fixes these upstream (https://github.com/ixmilia/dxf-rs).
