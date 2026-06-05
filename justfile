# Coverage for rabex-cli only — runs this package's tests and reports just its
# own sources (rabex-env and registry deps are compiled but filtered out).
coverage *args:
    cargo llvm-cov --package rabex-cli --ignore-filename-regex rabex-env {{args}}
