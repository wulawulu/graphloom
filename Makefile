build:
	@cargo build

test:
	@cargo nextest run --all-features

test-cli:
	@cargo nextest run -p graphloom --test cli_integration

test-api:
	@cargo nextest run -p graphloom --test api_index --test api_query

test-integration:
	@cargo nextest run -p graphloom --test cli_integration --test api_index --test api_query

test-compat:
	@cargo build -p graphloom
	@cargo build -p graphloom-vectors --example compat_vector_manifest
	@cargo build -p graphloom-storage --example compat_table_reader
	@cargo test -p graphloom-vectors --example compat_vector_manifest
	@env -u PYTHONPATH uv run --project tests/compat --locked ruff format --check tests/compat
	@env -u PYTHONPATH uv run --project tests/compat --locked ruff check tests/compat
	@TARGET_DIR="$$(cargo metadata --no-deps --format-version 1 | \
		sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"; \
	env -u PYTHONPATH PYTHONNOUSERSITE=1 \
		GRAPHLOOM_BIN="$$TARGET_DIR/debug/graphloom" \
		GRAPHLOOM_VECTOR_MANIFEST_BIN="$$TARGET_DIR/debug/examples/compat_vector_manifest" \
		GRAPHLOOM_TABLE_READER_BIN="$$TARGET_DIR/debug/examples/compat_table_reader" \
		uv run --project tests/compat --locked \
		pytest -q tests/compat
	@cargo test -p graphloom-llm --test cache_compat

test-query-record-replay:
	@env -u PYTHONPATH PYTHONNOUSERSITE=1 \
		uv run --project tests/compat --locked \
		pytest -q tests/compat/test_llm_cache_proxy.py tests/compat/test_query_record_replay.py

llm-cache-proxy:
	@env -u PYTHONPATH PYTHONNOUSERSITE=1 \
		uv run --project tests/compat --locked \
		python tests/compat/llm_cache_proxy.py \
		--cassette "$(CASSETTE)" \
		--completion-provider "$(or $(COMPLETION_PROVIDER),deepseek)" \
		--embedding-provider "$(or $(EMBEDDING_PROVIDER),ollama)" \
		$(if $(COMPLETION_API_BASE),--completion-api-base "$(COMPLETION_API_BASE)",) \
		$(if $(EMBEDDING_API_BASE),--embedding-api-base "$(EMBEDDING_API_BASE)",)

query-record-replay:
	@TARGET_DIR="$$(cargo metadata --no-deps --format-version 1 | \
		sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"; \
	env -u PYTHONPATH PYTHONNOUSERSITE=1 \
		uv run --project tests/compat --locked \
		python tests/compat/query_record_replay.py \
		--case "$(CASE)" \
		--query "$(QUERY)" \
		--method "$(or $(METHOD),all)" \
		--graphloom-bin "$$TARGET_DIR/debug/graphloom"

update-debug-audit:
	@../graphrag/.venv/bin/python update_debug/audit_update_fixture.py \
		--stage "$(or $(STAGE),preflight)"

test-update-debug-audit:
	@env -u PYTHONPATH PYTHONNOUSERSITE=1 \
		uv run --project tests/compat --locked \
		pytest -q update_debug/test_audit_update_fixture.py

update-reference-config-gate:
	@TARGET_DIR="$$(cargo metadata --no-deps --format-version 1 | \
		sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"; \
	../graphrag/.venv/bin/python update_reference/effective_config_gate.py \
		--graphloom-root update_reference/template \
		--graphrag-root ../graphrag/update_reference/template \
		--graphloom-bin "$$TARGET_DIR/debug/graphloom" \
		--output update_reference/artifacts/effective-config.json

test-all:
	@cargo nextest run --all-features

bench-query:
	@cargo test --workspace --all-features performance -- --ignored --nocapture

release:
	@cargo release tag --execute
	@git cliff -o CHANGELOG.md
	@git commit -a -n -m "Update CHANGELOG.md" || true
	@git push origin main
	@cargo release push --execute

update-submodule:
	@git submodule update --init --recursive --remote

.PHONY: build test test-cli test-api test-integration test-compat test-query-record-replay llm-cache-proxy query-record-replay update-debug-audit test-update-debug-audit update-reference-config-gate test-all bench-query release update-submodule
