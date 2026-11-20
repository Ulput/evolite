# How to check the evolite installer

По-русски: [verify.ru.md](verify.ru.md).

evolite's installers are **not code-signed**. Windows SmartScreen and macOS Gatekeeper warn about them, and a warning cannot tell a real installer from a fake one. So every release comes with:

- **`SHA256SUMS`**: the SHA-256 of each installer.
- **`SHA256SUMS.minisig`**: a [minisign](https://jedisct1.github.io/minisign/) signature of that list, made offline with the evolite release key.

Check both before you install. It takes a few minutes, once per release.

## 1. Download

From the release page (`https://github.com/Ulput/evolite/releases`), download into one folder:

- your installer, for example `evolite-setup-windows-x86_64.exe`;
- `SHA256SUMS`;
- `SHA256SUMS.minisig`.

## 2. Get the public key, and compare it in two places

The evolite release public keys (a signature by either one is valid):

```
main:   <published with the first signed release>
backup: <published with the first signed release>
```

Compare the key you use with at least **two** of these places:

- this page, in the repository;
- the repository's `README.md`;
- an installed evolite: **Settings › About** shows both keys;
- <a place outside GitHub, named with the first signed release>.

If they differ, stop: do not install.

## 3. Check the signature of `SHA256SUMS`

Install minisign from its official source, `https://github.com/jedisct1/minisign/releases`. Check minisign's own download by the sums published there.

```
minisign -Vm SHA256SUMS -P <main key>
```

It must say `Signature and comment signature verified`. If it fails, try the backup key. If both fail, stop.

## 4. Check the installer's sum

Compare your installer's SHA-256 with its line in `SHA256SUMS`.

- **Windows (PowerShell):** `Get-FileHash .\evolite-setup-windows-x86_64.exe -Algorithm SHA256`, or `certutil -hashfile evolite-setup-windows-x86_64.exe SHA256`
- **macOS:** `shasum -a 256 evolite-setup-macos-*.dmg`
- **Linux:** `sha256sum -c --ignore-missing SHA256SUMS`

The sum must match exactly, every character.

## 5. If something does not match

Do not install, and do not enter your recovery phrase anywhere. Delete the downloaded files and report it (<where, named with the first signed release>).

## 6. The system's warning, after the check passed

- **Windows:** SmartScreen says «Windows protected your PC». Choose **More info › Run anyway**.
- **macOS:** right-click the installer › **Open**, then **Open** again.

The warning is there because the installer is not code-signed; the check above is what tells you it is evolite's.

## In the app

evolite checks once a day for a newer version, if **Settings › Privacy › Check for a new version** is on. It only believes a notice signed with the keys above. It never downloads or installs anything by itself: it links here and to the release page.
