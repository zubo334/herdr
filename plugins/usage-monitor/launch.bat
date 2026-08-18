@echo off
set "PLUGIN_DIR=%HERDR_PLUGIN_ROOT%"
if "%PLUGIN_DIR:~0,4%"=="\\?\" set "PLUGIN_DIR=%PLUGIN_DIR:~4%"
cd /d "%PLUGIN_DIR%"
node dashboard.js
