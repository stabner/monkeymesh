@echo off
REM This is a TEMPLATE folder — the real miner lives in Releases\CpuMiner
cd /d "%~dp0"
set "RELEASE=%~dp0..\..\..\Releases\CpuMiner"
if exist "%RELEASE%\MonkeyMesh-CpuMiner.exe" (
  echo Opening the real CPU miner pack:
  echo   %RELEASE%
  echo.
  cd /d "%RELEASE%"
  call "%RELEASE%\Start-CpuMiner.bat"
  exit /b %ERRORLEVEL%
)
echo.
echo You started Start-CpuMiner.bat from the TEMPLATES folder.
echo That folder has no .exe — use this instead:
echo.
echo   Releases\CpuMiner\Start-CpuMiner.bat
echo.
echo Or run:  Launchers\stage-miner-releases.ps1
echo.
pause
exit /b 1
