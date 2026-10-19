# Fixture: scanned-rejection (no text layer)

This fixture documents the explicit rejection behaviour for a PDF with no usable
text layer (a scan). It asserts the documented rejection, not text extraction.

- reference_sha: `d72fb398e8959eea83ed24be03f6df8cdeca86a2`
- Expected behaviour: the RAG-007 PDF loader rejects the file explicitly because
  no text layer is present. OCR (RAG-008) is not implemented and is out of scope
  for this migration (see `docs/requirements/PRD-Phase-23b.md`).
- Capture command:
  `make ingest FILE=docs/reports/rust-migration/fixtures/page/scanned.pdf`
- Capture outcome: specification freeze. The end-to-end command was not run in the
  capture environment (database required); the expected error behaviour is fixed
  from `docs/reference/capabilities.md` and is an exact-match assertion (rejection,
  not extraction).
