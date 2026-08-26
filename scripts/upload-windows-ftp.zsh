#!/usr/bin/env zsh
# Upload the verified Windows release pair and provenance manifest over FTP.
#
# Credentials are deliberately kept in the ignored project-root
# `.ftp-deploy.env` file, or supplied as VEYR_FTP_* environment variables.
set -euo pipefail

SCRIPT_DIR="${0:A:h}"
PROJECT_ROOT="${SCRIPT_DIR:h}"
DIST_DIR="${VEYR_DIST_DIR:-$PROJECT_ROOT/dist}"
CONFIG_FILE="${VEYR_FTP_CONFIG:-$PROJECT_ROOT/.ftp-deploy.env}"

if [[ -f "$CONFIG_FILE" ]]; then
    source "$CONFIG_FILE"
fi

: "${VEYR_FTP_URL:?Set VEYR_FTP_URL or create .ftp-deploy.env}"
: "${VEYR_FTP_USER:?Set VEYR_FTP_USER or create .ftp-deploy.env}"
: "${VEYR_FTP_PASSWORD:?Set VEYR_FTP_PASSWORD or create .ftp-deploy.env}"
: "${VEYR_FTP_REMOTE_DIR:=/}"

"$SCRIPT_DIR/verify-windows-artifacts.zsh" "$DIST_DIR"

for artifact in veyr.dll veyr.exe manifest.json; do
    [[ -f "$DIST_DIR/$artifact" ]] || {
        print -u2 -- "Missing verified artifact: $DIST_DIR/$artifact"
        exit 1
    }
done

remote_dir="${VEYR_FTP_REMOTE_DIR%/}"
[[ -n "$remote_dir" ]] || remote_dir="/"

upload() {
    local artifact="$1"
    local final_path="${remote_dir}/${artifact}"
    # This server does not allow replacing a file through RNTO. Uploading the
    # binary pair before the manifest still gives consumers a clear commit
    # point: they accept a release only after the matching manifest arrives.
    curl --fail --silent --show-error --ftp-create-dirs --ftp-pasv \
        --user "$VEYR_FTP_USER:$VEYR_FTP_PASSWORD" \
        --upload-file "$DIST_DIR/$artifact" \
        "${VEYR_FTP_URL%/}${final_path}"
    print -- "FTP uploaded: $artifact"
}

upload veyr.dll
upload veyr.exe
upload manifest.json
print -- "FTP deployment complete: ${VEYR_FTP_URL%/}${remote_dir}/"
