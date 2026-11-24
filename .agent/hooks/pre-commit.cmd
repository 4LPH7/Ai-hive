@echo off
REM Windows pre-commit automated quality gate for ai-hive
echo === [1/3] Running hive lint ===
python hive.py lint
if %ERRORLEVEL% NEQ 0 exit /b %ERRORLEVEL%

echo === [2/3] Running Python regression tests ===
python -m unittest discover
if %ERRORLEVEL% NEQ 0 exit /b %ERRORLEVEL%

echo === [3/3] Running Rust test suite ===
cargo test --quiet
if %ERRORLEVEL% NEQ 0 exit /b %ERRORLEVEL%

echo All quality checks passed clean.
exit /b 0
