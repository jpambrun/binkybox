set shell := ["powershell.exe", "-NoLogo", "-Command"]

kill:
	Get-Process -Name binkybox -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue; exit 0

build: kill
	cargo build -r

run: kill
	& ".\\target\\release\\binkybox.exe"

buildrun: build run
