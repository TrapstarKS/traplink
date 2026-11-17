# Local Logi Options+ check

File examined: `C:\Program Files\LogiOptionsPlus\logioptionsplus.exe`.

On 2026-11-15, static triage identified a Windows x86-64 PE executable. Its resources report Logi Options+, publisher Logitech, file version 2.9.3853.0, and an Electron ASAR integrity resource. The manifest requests `asInvoker`, so the UI process does not elevate automatically.

Windows Authenticode verification returned **Valid**, signed by **Logitech Inc** with a DigiCert timestamp. SHA-256: `746935A1E973AAAFE810C5D3C190998D8E9A827147C390C18D1706198FEE8C8A`.

This supports the authenticity and integrity of this signed executable. It does not certify the absence of vulnerabilities, telemetry or issues in helper services. The static analysis was partial (bounded functions and strings) and did not run or modify the program. No full audit of all Options+ files or services was done.

Tools: a static PE analysis CLI (`commands`, `triage`, `native-pe-resources-list`, `core-overview-show`), `Get-AuthenticodeSignature` and `Get-FileHash`. JSON evidence is kept in the git-ignored local `artifacts` directory.
