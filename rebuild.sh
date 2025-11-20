#!/bin/bash

docker run --rm --platform linux/amd64 -v "$PWD":/app -w /app rust:latest cargo build --release