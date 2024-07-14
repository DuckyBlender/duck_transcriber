#!/bin/bash

set -e

# Define variables
FILE="ffmpeg-release-arm64-static.tar.xz"
FOLDER="ffmpeg-release-arm64-static"
URL="https://johnvansickle.com/ffmpeg/releases/$FILE"

# Check if tar.xz file exists
if [ -f "$FILE" ]; then
    echo "$FILE exists, skipping download."
else
    echo "Downloading $FILE..."
    wget $URL
    echo "Download complete."
fi

# Check if tar.xz file is extracted
if [ -d "$FOLDER" ]; then
    echo "$FOLDER exists, skipping extraction."
else
    echo "Extracting $FILE..."
    tar xf $FILE --transform 's!^[^/]\+\($\|/\)!ffmpeg-release-arm64-static\1!'
    echo "Extraction complete."
fi

# Check if ffmpeg binary is already moved
if [ -f "$FOLDER/ffmpeg" ]; then
    echo "Moving ffmpeg binary to root folder..."
    mv $FOLDER/ffmpeg .
    echo "Move complete."
else
    echo "ffmpeg binary already in root folder, skipping move."
fi

# Build and deploy
echo "Building and deploying..."
cargo lambda build --release --arm64
cargo lambda deploy
echo "Build and deploy complete."