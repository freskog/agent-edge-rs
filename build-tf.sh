#!/usr/bin/env bash
set -euo pipefail

TFLITE_TAG="v2.14.0"                       # change here if you want a newer tag
ARCH_DIR="libs/linux-aarch64"              # where Rust build.rs looks for the .so’s
DEST_DIR="$(pwd)/${ARCH_DIR}"              # absolute path
NUM_JOBS=$(nproc)

# --------------------------------------------------------------------------
# 0. Install Bazelisk if it is missing
# --------------------------------------------------------------------------
if ! command -v bazelisk >/dev/null 2>&1; then
  echo "▶ Installing bazelisk …"
  BZ_VERS="v1.19.0"
  curl -L \
    "https://github.com/bazelbuild/bazelisk/releases/download/${BZ_VERS}/bazelisk-linux-arm64" \
    -o bazelisk
  chmod +x bazelisk
  sudo mv bazelisk /usr/local/bin/
fi

# --------------------------------------------------------------------------
# 1. Clone TensorFlow at the requested tag
# --------------------------------------------------------------------------
echo "▶ Cloning TensorFlow $TFLITE_TAG"
rm -rf tensorflow
git clone --depth 1 --branch "${TFLITE_TAG}" https://github.com/tensorflow/tensorflow.git
cd tensorflow

# --------------------------------------------------------------------------
# 2. Build the TF-Lite shared libraries with XNNPACK enabled
# --------------------------------------------------------------------------
echo "▶ Building TensorFlow-Lite with XNNPACK (${NUM_JOBS} parallel jobs)"
bazelisk clean --expunge
bazelisk build -c opt --jobs="${NUM_JOBS}" \
  --config=monolithic \
  --define tflite_with_xnnpack=true \
  //tensorflow/lite:libtensorflowlite.so \
  //tensorflow/lite/c:libtensorflowlite_c.so

# --------------------------------------------------------------------------
# 3. Copy / symlink the artefacts into the Rust project
# --------------------------------------------------------------------------
echo "▶ Copying artefacts into ${DEST_DIR}"
mkdir -p "${DEST_DIR}"
cp -f bazel-bin/tensorflow/lite/libtensorflowlite.so         "${DEST_DIR}/"
cp -f bazel-bin/tensorflow/lite/c/libtensorflowlite_c.so     "${DEST_DIR}/"

echo "✅  TensorFlow-Lite built and copied successfully"
echo "   Files:"
ls -lh "${DEST_DIR}"/*.so