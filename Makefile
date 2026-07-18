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
	@cargo test -p graphloom-vectors --example compat_vector_manifest
	@env -u PYTHONPATH uv run --project tests/compat --locked ruff format --check tests/compat
	@env -u PYTHONPATH uv run --project tests/compat --locked ruff check tests/compat
	@TARGET_DIR="$$(cargo metadata --no-deps --format-version 1 | \
		sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"; \
	env -u PYTHONPATH PYTHONNOUSERSITE=1 \
		GRAPHLOOM_BIN="$$TARGET_DIR/debug/graphloom" \
		GRAPHLOOM_VECTOR_MANIFEST_BIN="$$TARGET_DIR/debug/examples/compat_vector_manifest" \
		uv run --project tests/compat --locked \
		pytest -q tests/compat
	@cargo test -p graphloom-llm --test cache_compat

test-all:
	@cargo nextest run --all-features

check-agent-sync:
	@cmp -s CLAUDE.md AGENTS.md || { \
		echo "AGENTS.md must stay in sync with CLAUDE.md"; \
		echo "Update both files with the same shared project instructions."; \
		exit 1; \
	}
	@tmp_dir=$$(mktemp -d); \
	trap 'rm -rf "$$tmp_dir"' EXIT; \
	cp -R .claude/skills "$$tmp_dir/expected-skills"; \
	find "$$tmp_dir/expected-skills" -name SKILL.md -exec perl -0pi -e 's/CLAUDE\.md/AGENTS.md/g; s/Claude/Codex/g; s/claude/codex/g' {} +; \
	diff -ru --exclude agents "$$tmp_dir/expected-skills" .agents/skills || { \
		echo "Codex skills must stay in sync with Claude skills after Claude-to-Codex renaming."; \
		echo "Update .claude/skills first, then mirror the shared content into .agents/skills."; \
		exit 1; \
	}

release:
	@cargo release tag --execute
	@git cliff -o CHANGELOG.md
	@git commit -a -n -m "Update CHANGELOG.md" || true
	@git push origin main
	@cargo release push --execute

update-submodule:
	@git submodule update --init --recursive --remote

.PHONY: build test test-cli test-api test-integration test-compat test-all check-agent-sync release update-submodule
