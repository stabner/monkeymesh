@echo off
REM This is a TEMPLATE folder — the real miner lives in Releases\GpuMiner
cd /d "%~dp0"
set "RELEASE=%~dp0..\..\..\Releases\GpuMiner"
if exist "%RELEASE%\MonkeyMesh-GpuMiner.exe" (
  echo Opening the real GPU miner pack:
  echo   %RELEASE%
  echo.
  cd /d "%RELEASE%"
  call "%RELEASE%\Start-GpuMiner.bat"
  exit /b %ERRORLEVEL%
)
echo.
echo You started Start-GpuMiner.bat from the TEMPLATES folder.
echo That folder has no .exe — use this instead:
echo.
echo   Releases\GpuMiner\Start-GpuMiner.vbs
echo.
echo Or run:  Launchers\stage-miner-releases.ps1
echo.
pause
exit /b 1
