#!/bin/sh
# Shared settings for the SOP RAG scripts. Source this file, do not execute it.
#
# Every value is assigned unconditionally from its SOP_RAG_-prefixed override
# or its default. An inherited DATABASE_URL or OLLAMA_* value never wins. This
# is the opposite of scripts/env-defaults.sh, which keeps caller values.

: "${SOP_RAG_BIN_DIR:=$HOME/agentic-workspace/projects/rag-template-rust/bin}"
export SOP_RAG_BIN_DIR

: "${SOP_RAG_DATABASE_URL:=postgres://rag:rag@127.0.0.1:5436/rag?sslmode=disable}"
DATABASE_URL=$SOP_RAG_DATABASE_URL
export DATABASE_URL

: "${SOP_RAG_OLLAMA_URL:=http://localhost:11434}"
OLLAMA_URL=$SOP_RAG_OLLAMA_URL
export OLLAMA_URL

: "${SOP_RAG_OLLAMA_EMBED_MODEL:=nomic-embed-text}"
OLLAMA_EMBED_MODEL=$SOP_RAG_OLLAMA_EMBED_MODEL
export OLLAMA_EMBED_MODEL

: "${SOP_RAG_EMBED_DIM:=768}"
EMBED_DIM=$SOP_RAG_EMBED_DIM
export EMBED_DIM

: "${SOP_RAG_EMBED_PROVIDER:=ollama}"
EMBED_PROVIDER=$SOP_RAG_EMBED_PROVIDER
export EMBED_PROVIDER

: "${SOP_RAG_CORPUS:=tran-docs}"
RAG_CORPUS=$SOP_RAG_CORPUS
export RAG_CORPUS

: "${SOP_RAG_RETRIEVAL_ALLOWED_CORPORA:=tran-docs}"
RAG_RETRIEVAL_ALLOWED_CORPORA=$SOP_RAG_RETRIEVAL_ALLOWED_CORPORA
export RAG_RETRIEVAL_ALLOWED_CORPORA
