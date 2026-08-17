# 11 — Todo List with File Storage
**Tests:** file I/O, serialization, escaping, round trips. **Result:** works, all three engines agree.

The subject is not "does `write_file` work" but whether a value survives being
written and read back, so every case is a round trip compared field by field.

The escaping is the real content. A todo's text can contain the field delimiter,
a newline, or a backslash, and each breaks a naive line-per-record format. All
three are in the fixture deliberately, and both round trips preserve them.

## Findings
**F6** — files can be written and truncated, never removed, and a directory
cannot be listed. The program can only empty its scratch file, so `file_exists`
stays true forever.
