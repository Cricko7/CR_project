@echo off
setlocal EnableExtensions

cd /d "%~dp0"

if exist "docker-env.bat" (
  call "docker-env.bat"
  if errorlevel 1 (
    echo [ERROR] docker-env.bat returned non-zero exit code.
    exit /b 1
  )
) else (
  echo [INFO] docker-env.bat not found, using current shell environment.
)

docker compose version >nul 2>&1
if errorlevel 1 (
  echo [ERROR] docker compose is not available. Install Docker Desktop with Compose plugin.
  exit /b 1
)

echo [INFO] Rebuilding and recreating app containers...
docker compose up --build -d --force-recreate api worker frontend
if errorlevel 1 (
  echo [ERROR] docker compose restart workflow failed.
  exit /b 1
)

echo [INFO] Current services:
docker compose ps

endlocal
