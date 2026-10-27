# Install prek and register the git pre-commit hooks
hooks:
    uv tool install -U prek
    prek install

# Run all pre-commit hooks on every file
lint:
    prek run --all-files
