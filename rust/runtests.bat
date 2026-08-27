@echo off
cd /d C:\Users\David\Documents\metrum_rise\rust
set CARGO_PROFILE_TEST_DEBUG=0
set CARGO_INCREMENTAL=0
cargo test --lib --no-fail-fast > tests.log 2>&1
echo EXIT=%errorlevel% >> tests.log
