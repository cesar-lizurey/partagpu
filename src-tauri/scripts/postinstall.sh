#!/bin/bash
mkdir -p /usr/local/lib/partagpu
cp /usr/lib/PartaGPU/resources/partagpu-helper /usr/local/lib/partagpu/partagpu-helper
chmod 755 /usr/local/lib/partagpu/partagpu-helper
chown root:root /usr/local/lib/partagpu/partagpu-helper
cp /usr/lib/PartaGPU/resources/com.partagpu.policy /usr/share/polkit-1/actions/com.partagpu.policy
chmod 644 /usr/share/polkit-1/actions/com.partagpu.policy
