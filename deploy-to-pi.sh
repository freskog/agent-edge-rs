#!/bin/bash
set -e

# ─────────────────────────────
# 🎨 Colors
# ─────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# ─────────────────────────────
# 🧾 Args
# ─────────────────────────────
FULL_DEPLOY=false
PI_TARGET=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --full) FULL_DEPLOY=true; shift ;;
        *) PI_TARGET="$1"; shift ;;
    esac
done

if [[ -z "$PI_TARGET" || ! "$PI_TARGET" =~ "@" ]]; then
    echo -e "${RED}Usage:${NC} $0 [--full] user@host"
    exit 1
fi

PI_USER="${PI_TARGET%@*}"
REMOTE_DIR="/home/${PI_USER}/agent-edge"

echo -e "${BLUE}📦 Deploying to ${PI_TARGET}${NC}"
echo -e "Mode: $([ "$FULL_DEPLOY" = true ] && echo "Full" || echo "Quick")"
echo ""

# ─────────────────────────────
# 🔨 Build
# ─────────────────────────────
echo -e "${YELLOW}🔧 Building binary (release)...${NC}"
cargo build
BIN_PATH="target/release/agent-edge"

if [ ! -f "$BIN_PATH" ]; then
    echo -e "${RED}❌ Build failed${NC}"
    exit 1
fi

# ─────────────────────────────
# 🧪 Check .so presence
# ─────────────────────────────
LIB_DIR="libs/linux-aarch64"
if [ ! -f "$LIB_DIR/libtensorflowlite_c.so" ] || [ ! -f "$LIB_DIR/libtensorflowlite.so" ]; then
    echo -e "${RED}❌ Missing .so files in $LIB_DIR${NC}"
    exit 1
fi

# ─────────────────────────────
# 📁 Optional: check models
# ─────────────────────────────
if $FULL_DEPLOY; then
    echo -e "${YELLOW}📁 Checking models...${NC}"
    REQUIRED_MODELS=(
        "models/melspectrogram.tflite"
        "models/embedding_model.tflite"
        "models/hey_mycroft_v0.1.tflite"
    )

    for m in "${REQUIRED_MODELS[@]}"; do
        [ -f "$m" ] || { echo -e "${RED}❌ Missing model: $m${NC}"; exit 1; }
    done
    echo -e "${GREEN}✅ Models present${NC}"
fi

# ─────────────────────────────
# 🌐 SSH Test
# ─────────────────────────────
echo -e "${YELLOW}🔌 Testing SSH to $PI_TARGET...${NC}"
ssh -o ConnectTimeout=5 "$PI_TARGET" true || {
    echo -e "${RED}❌ SSH failed${NC}"; exit 1;
}
echo -e "${GREEN}✅ SSH OK${NC}"

# ─────────────────────────────
# 🚀 Upload
# ─────────────────────────────
echo -e "${YELLOW}📤 Uploading files...${NC}"
ssh "$PI_TARGET" "mkdir -p $REMOTE_DIR/libs/linux-aarch64"

scp "$BIN_PATH" "$PI_TARGET:$REMOTE_DIR/"
scp "$LIB_DIR"/*.so "$PI_TARGET:$REMOTE_DIR/libs/linux-aarch64/"

if $FULL_DEPLOY; then
    ssh "$PI_TARGET" "mkdir -p $REMOTE_DIR/models"
    scp models/*.tflite "$PI_TARGET:$REMOTE_DIR/models/"
fi

ssh "$PI_TARGET" "chmod +x $REMOTE_DIR/agent-edge"

echo -e "${GREEN}✅ Files deployed${NC}"

# ─────────────────────────────
# 🧪 Verify
# ─────────────────────────────
echo -e "${YELLOW}🧪 Testing remote binary...${NC}"
if ssh "$PI_TARGET" "$REMOTE_DIR/agent-edge --help" &>/dev/null; then
    echo -e "${GREEN}✅ Binary runs successfully on the Pi${NC}"
else
    echo -e "${RED}⚠️  Binary failed to run (check missing libs or arch mismatch)${NC}"
    exit 1
fi

# ─────────────────────────────
# 🏁 Done
# ─────────────────────────────
echo ""
echo -e "${GREEN}🎉 Deploy complete${NC}"
echo -e "Run it on the Pi:"
echo -e "${BLUE}  ssh $PI_TARGET${NC}"
echo -e "${BLUE}  cd $REMOTE_DIR && ./agent-edge${NC}"
