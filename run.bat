@echo off
cd /d "%~dp0"
start "Rustify API" cmd /k bacon api
bacon client
