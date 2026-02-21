set shell := ["powershell.exe", "-NoLogo", "-Command"]
set script-interpreter := ['bun', 'run']

kill:
	@Get-Process -Name binkybox -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue; exit 0

build: kill
	@cargo build -r

run: kill
	& ".\\target\\release\\binkybox.exe"

buildrun: build run

[script]
release: build
	import { $, file } from "bun"
	const repo = "jpambrun/binkybox"
	const asset = "target/release/binkybox.exe"
	const now = new Date()
	const title = now.toISOString().replace("T", " ").replace("Z", " UTC")
	const tag = `release-${now.toISOString().replace(/[-:]/g, "").replace("T", "-").replace(".000Z", "")}`
	await $`gh release create ${tag} ${asset}#binkybox.exe --title ${title} --notes "Automated release from just release." --repo ${repo}`
