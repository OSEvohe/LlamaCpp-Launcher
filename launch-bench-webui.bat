@echo off
setlocal

set "ROOT=%~dp0"
cd /d "%ROOT%"

:loop
python "bench\webui.py" --host 0.0.0.0
if errorlevel 42 if not errorlevel 43 goto loop

endlocal
