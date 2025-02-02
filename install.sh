#!/bin/bash

echo -e "\033[1;33m=== Solana Pump Monitor 安装 ===\033[0m"

# 检查并安装必要的工具
echo "检查并安装必要工具..."
if ! command -v wget &> /dev/null; then
    echo "安装 wget..."
    apt-get update
    apt-get install wget -y
fi

# 切换到root目录
cd ~

# 下载主要文件
echo "下载监控脚本..."
wget https://raw.githubusercontent.com/fcrre26/monitor/refs/heads/main/monitor.rs -O monitor.rs
wget https://raw.githubusercontent.com/fcrre26/monitor/refs/heads/main/monitor.sh -O monitor.sh
wget https://raw.githubusercontent.com/fcrre26/monitor/refs/heads/main/Cargo.toml -O Cargo.toml

# 设置执行权限
echo "设置权限..."
chmod +x monitor.sh

# 创建必要的目录和文件
echo "创建配置目录..."
mkdir -p ~/.solana_pump
echo '{"addresses":[]}' > ~/.solana_pump/watch_addresses.json

# 配置DNS
echo "配置DNS..."
echo "nameserver 8.8.8.8" >> /etc/resolv.conf
echo "nameserver 1.1.1.1" >> /etc/resolv.conf

echo "安装完成！"
echo "开始运行监控程序..."

# 运行脚本
./monitor.sh
