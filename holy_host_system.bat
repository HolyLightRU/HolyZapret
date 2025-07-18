@echo off
cd /d "%~dp0"
chcp 65001 > nul
title Прокидываю хосты

set "BIN=%~dp0bin\"
set "LISTS=%~dp0lists\"

NET SESSION >nul 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo Запрос на получение административных прав...
    powershell -Command "Start-Process -FilePath '%~dpnx0' -Verb RunAs"
    exit /b
)
powershell -Command "Write-Host 'Добавляю записи в файл hosts...' -ForegroundColor Yellow"
set "HOSTS_FILE=C:\Windows\System32\drivers\etc\hosts"
set "HOSTS_LIST=%LISTS%hosts-list.txt"  :: Укажите имя вашего файла со списком хостов

powershell -Command "Copy-Item '%HOSTS_FILE%' '%HOSTS_FILE%.bak' -Force" >nul 2>&1

for /f "tokens=*" %%a in ('type "%HOSTS_LIST%"') do (
    find /i "%%a" "%HOSTS_FILE%" >nul 2>&1 || (
        echo %%a >> "%HOSTS_FILE%"
        powershell -Command "Write-Host 'Добавлено: %%a' -ForegroundColor Magenta"
    )
)

:skip_hosts