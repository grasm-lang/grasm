# Test files somebody else wrote

Every file here was published by the body that defines the format it tests,
and none of it was written in this project. A gate that checks an
implementation against expectations invented beside it checks that the
author was consistent, which is not the same as checking that the
implementation is right — four fabricated vectors have already been caught
here, and the code was correct every time.

| File | From | Used by |
|---|---|---|
| `uritemplate-*.json` | the `uritemplate-test` suite maintained by the RFC 6570 authors | `urit_diff.sh` |
| `jsonpatch-tests.json`, `jsonpatch-spec-tests.json` | the `json-patch/json-patch-tests` suite | `patch_diff.sh` |
| `rfc7386-appendix-a.txt` | appendix A of RFC 7386, verbatim | `patch_diff.sh` |
| `GraphemeBreakTest.txt`, `WordBreakTest.txt`, `SentenceBreakTest.txt` | the Unicode Consortium, UCD 17.0.0 | `segment_diff.sh` |
| `NormalizationTest.txt` | the Unicode Consortium, UCD 17.0.0 | `normalise_diff.sh` |
| `bip39-vectors.json` | the BIP-39 reference implementation | `bip39_diff.sh` |
| `h2-transcript.txt` | recorded from the Python `h2` library by `h2_capture.py` | `h2_diff.sh` |

The Unicode files carry their own terms of use, which are in the files.
