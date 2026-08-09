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

prompt-tune-real-llm-check:
	@env -u PYTHONPATH PYTHONNOUSERSITE=1 \
		uv run --project tests/compat --locked \
		python tests/compat/prompt_tune_real_llm.py \
		--check \
		--settings "$(or $(SETTINGS),debug/settings.yaml)" \
		--env-file "$(or $(ENV_FILE),debug/.env)" \
		--graphrag-source "$(or $(GRAPHRAG_SOURCE),../graphrag)"

prompt-tune-real-llm-run: build
	@TARGET_DIR="$$(cargo metadata --no-deps --format-version 1 | \
		sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"; \
	env -u PYTHONPATH PYTHONNOUSERSITE=1 \
		uv run --project tests/compat --locked \
		python tests/compat/prompt_tune_real_llm.py \
		--run \
		--settings "$(or $(SETTINGS),debug/settings.yaml)" \
		--env-file "$(or $(ENV_FILE),debug/.env)" \
		--graphrag-source "$(or $(GRAPHRAG_SOURCE),../graphrag)" \
		--graphloom-bin "$$TARGET_DIR/debug/graphloom" \
		--selection-method "$(or $(SELECTION_METHOD),top)" \
		--input-dir "$(or $(INPUT_DIR),tests/compat/fixtures/prompt_tune/top/input)" \
		--input-file-pattern "$(or $(INPUT_FILE_PATTERN),.*[.]txt)" \
		--limit "$(or $(LIMIT),3)" \
		--chunk-size "$(or $(CHUNK_SIZE),38)" \
		--overlap "$(or $(OVERLAP),0)" \
		--encoding-model "$(or $(ENCODING_MODEL),o200k_base)" \
		--n-subset-max "$(or $(N_SUBSET_MAX),300)" \
		--k "$(or $(K),15)" \
		--no-discover-entity-types \
		--run-name "$(or $(RUN_NAME),prompt-tune-real-llm)" \
		$(if $(CLEAN),--clean,)

prompt-tune-update-debug: INPUT_DIR = update_debug/input
prompt-tune-update-debug: CHUNK_SIZE = 1200
prompt-tune-update-debug: OVERLAP = 100
prompt-tune-update-debug: RUN_NAME = update-debug-top
prompt-tune-update-debug: prompt-tune-real-llm-run

prompt-tune-random-real-llm: SELECTION_METHOD = random
prompt-tune-random-real-llm: INPUT_FILE_PATTERN = first[.]txt
prompt-tune-random-real-llm: LIMIT = 1
prompt-tune-random-real-llm: CHUNK_SIZE = 1000
prompt-tune-random-real-llm: N_SUBSET_MAX = 1
prompt-tune-random-real-llm: K = 1
prompt-tune-random-real-llm: RUN_NAME = random-single-candidate
prompt-tune-random-real-llm: prompt-tune-real-llm-run

prompt-tune-auto-real-llm: SELECTION_METHOD = auto
prompt-tune-auto-real-llm: INPUT_FILE_PATTERN = first[.]txt
prompt-tune-auto-real-llm: LIMIT = 1
prompt-tune-auto-real-llm: CHUNK_SIZE = 1000
prompt-tune-auto-real-llm: N_SUBSET_MAX = 1
prompt-tune-auto-real-llm: K = 1
prompt-tune-auto-real-llm: RUN_NAME = auto-single-candidate
prompt-tune-auto-real-llm: prompt-tune-real-llm-run

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

studio-build:
	@cd studio && pnpm build

studio-check:
	@cd studio && pnpm lint
	@cd studio && pnpm typecheck
	@cd studio && pnpm run test --run
	@cd studio && pnpm build

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

.PHONY: build test test-cli test-api test-integration test-compat test-query-record-replay prompt-tune-real-llm-check prompt-tune-real-llm-run prompt-tune-update-debug prompt-tune-random-real-llm prompt-tune-auto-real-llm llm-cache-proxy query-record-replay update-debug-audit test-update-debug-audit update-reference-config-gate test-all studio-build studio-check bench-query release update-submodule
