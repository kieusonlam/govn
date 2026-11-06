@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
cd /d %~dp0
if not exist out mkdir out
cl /nologo /EHsc /O1 /W0 /utf-8 /FIshim.h /I. /DUNIKEYHOOK /D_CRT_SECURE_NO_WARNINGS /D_CRT_NONSTDC_NO_WARNINGS /I..\..\ibus-unikey\ukengine /Foout\ /Feout\oracle.exe oracle.cpp ..\..\ibus-unikey\ukengine\byteio.cpp ..\..\ibus-unikey\ukengine\charset.cpp ..\..\ibus-unikey\ukengine\convert.cpp ..\..\ibus-unikey\ukengine\data.cpp ..\..\ibus-unikey\ukengine\error.cpp ..\..\ibus-unikey\ukengine\inputproc.cpp ..\..\ibus-unikey\ukengine\mactab.cpp ..\..\ibus-unikey\ukengine\pattern.cpp ..\..\ibus-unikey\ukengine\ukengine.cpp ..\..\ibus-unikey\ukengine\unikey.cpp ..\..\ibus-unikey\ukengine\usrkeymap.cpp
