@echo off
cd /d "%~dp0"
chcp 65001 >nul
title Прокидываю хосты

setlocal EnableDelayedExpansion

set "BIN=%~dp0bin\"
set "LISTS=%~dp0lists\"
set "HOSTS_FILE=C:\Windows\System32\drivers\etc\hosts"
set "HOSTS_LIST=%LISTS%hosts-list.txt"
set "TEMP_FILE=%TEMP%\hosts.tmp"

NET SESSION >nul 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo Запрос на получение административных прав...
    powershell -Command "Start-Process -FilePath '%~dpnx0' -Verb RunAs"
    exit /b
)

powershell -Command "Write-Host 'Обновляю записи в файле hosts...' -ForegroundColor Yellow"

powershell -Command "Copy-Item -Path '%HOSTS_FILE%' -Destination '%HOSTS_FILE%.bak' -Force" >nul 2>&1

type nul > "%TEMP_FILE%"

set "processed_domains="
for /f "usebackq delims=" %%L in ("%HOSTS_FILE%") do (
    set "line=%%L"
    set "keep_line=1"
    set "current_domain="
    
    echo "!line!" | findstr /i "^#.*" >nul && set "keep_line=1" && goto :process_line
    echo "!line!" | findstr /i "^[[:space:]]*$" >nul && set "keep_line=1" && goto :process_line
    
    for /f "tokens=1,*" %%a in ("!line!") do (
        for %%d in (%%b) do set "current_domain=%%d"
    )
    
    if defined current_domain (
        findstr /i /c:"!current_domain!" "%HOSTS_LIST%" >nul && set "keep_line=0"
        for %%d in (!processed_domains!) do (
            if /i "%%d"=="!current_domain!" set "keep_line=0"
        )
        
        if "!keep_line!"=="1" (
            set "processed_domains=!processed_domains! !current_domain!"
        )
    )
    
    :process_line
    if "!keep_line!"=="1" (
        echo !line!>> "%TEMP_FILE%"
    )
)

for /f "usebackq tokens=1,2" %%A in ("%HOSTS_LIST%") do (
    set "new_ip=%%A"
    set "domain=%%B"
    
    findstr /i /c:"!domain!" "%HOSTS_FILE%" >nul
    if !errorlevel! equ 0 (
        powershell -Command "Write-Host 'Обновляю: !domain! -> !new_ip!' -ForegroundColor Cyan"
    ) else (
        powershell -Command "Write-Host 'Добавлено: !new_ip! !domain!' -ForegroundColor Magenta"
    )
    
    echo !new_ip! !domain!>> "%TEMP_FILE%"
)

move /y "%TEMP_FILE%" "%HOSTS_FILE%" >nul 2>&1

powershell -Command "Write-Host 'Готово!' -ForegroundColor Green"
endlocal
exit /b