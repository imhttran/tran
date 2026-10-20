# TRAN-001 — Genealogy Document Inventory

Status: PROPOSED
Project: Tran Family Tree RAG
Repository: imhttran/tran
Runtime: Rust, PostgreSQL/pgvector, Ollama
Workflow: SOP-managed

## Project

Tran Family Tree RAG

## Summary

Establish a verified source inventory for the Tran family
genealogy project.

Reuse existing transcription, translation, and catalog work.
Preserve source provenance and protect private family records.

## Source Directory

docs/tran/

Expected files:

- Family Genealogy.pdf
- Tran-Family-Vietnamese.md
- Tran-Family-English.md
- Tran-Family-People-Catalog.md

## TRAN-001A — Source Inventory

- Discover all source files recursively.
- Record relative path, format, size, and SHA-256 hash.
- Count PDF pages.
- Detect missing or duplicate source files.
- Write a local manifest.

### Acceptance Criteria

- Every discovered file has an inventory record.
- Manifest hashes match the source files.
- No original source is modified.

## TRAN-001B — PDF Assessment

- Inspect the PDF text layer.
- Determine which pages need OCR.
- Check existing Vietnamese transcription coverage.
- Identify missing, unreadable, or uncertain pages.
- Do not repeat OCR for verified existing text.

### Acceptance Criteria

- Every PDF page has an assessment.
- OCR gaps are listed with page references.
- Original text is preserved.

## TRAN-001C — Translation Assessment

- Compare Vietnamese and English Markdown files.
- Check section and page alignment.
- Identify missing or uncertain translations.
- Preserve Vietnamese personal names and source spelling.
- Do not regenerate verified translations.

### Acceptance Criteria

- Translation coverage is documented.
- All identified gaps have source references.
- No unsupported translation is treated as verified.

## TRAN-001D — People Catalog Assessment

- Inspect the existing people catalog.
- Identify people, alternate names, dates, and places.
- Inspect documented family relationships.
- Identify duplicate and ambiguous identities.
- Require source evidence for each verified relationship.

### Acceptance Criteria

- Catalog coverage is documented.
- Missing provenance is identified.
- Uncertain relationships remain unverified.
- No family relationship is invented.

## TRAN-001E — Final Evidence

Create local outputs under:

docs/tran/audit/

Required:

- manifest.json
- TRAN-001-report.md

The report must include:

- Document inventory and checksums
- PDF page and OCR assessment
- Translation coverage
- Catalog coverage
- Provenance gaps
- Recommended TRAN-002 scope

### Dependencies

- TRAN-001A
- TRAN-001B
- TRAN-001C
- TRAN-001D

### Deliverables

- docs/tran/audit/manifest.json
- docs/tran/audit/TRAN-001-report.md

### Acceptance Criteria

- All discovered documents are inventoried.
- Source hashes are recorded and verified.
- PDF extraction coverage is assessed.
- Existing translation coverage is assessed.
- Existing catalog coverage is assessed.
- Source provenance gaps are reported.
- Git privacy protection is verified.
- All required local evidence files exist.
- No original source document has changed.

## Safety

- Treat docs/tran/ as private, read-only source data.
- Confirm Git ignores docs/tran/.
- Never commit private documents or detailed audit outputs.
- Do not overwrite source documents.
- Do not modify production database records.
- Use disposable resources for integration tests.
- Stop for required authorization or a real blocker.

## Final Acceptance

TRAN-001 passes only when:

1. All discovered documents are inventoried.
2. Source hashes are recorded and verified.
3. PDF extraction coverage is assessed.
4. Existing translation coverage is assessed.
5. Existing catalog coverage is assessed.
6. Source provenance gaps are reported.
7. Git privacy protection is verified.
8. All required local evidence files exist.
9. No original source document has changed.

Record the SOP verdict and remaining gaps.
