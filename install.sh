#!/bin/bash

echo -e "\033[1;33m=== Solana Pump Monitor 安装 ===\033[0m"

# 工作目录
WORK_DIR="$HOME/workspace/pump"
INSTALL_DIR="$HOME/.solana_pump"

# 检查并安装必要的工具
echo "检查并安装必要工具..."
if ! command -v wget &> /dev/null; then
    echo "安装依赖..."
    apt-get update
    apt-get install wget build-essential pkg-config libssl-dev libudev-dev curl git jq bc -y
fi

# 安装 Rust
if ! command -v rustc &> /dev/null; then
    echo "安装 Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source $HOME/.cargo/env
fi

# 创建目录
mkdir -p $WORK_DIR
mkdir -p $INSTALL_DIR/{bin,logs,config}
cd $WORK_DIR

# 下载文件
echo "下载源文件..."
wget https://raw.githubusercontent.com/fcrre26/monitor/refs/heads/main/monitor.rs -O $WORK_DIR/monitor.rs
wget https://raw.githubusercontent.com/fcrre26/monitor/refs/heads/main/Cargo.toml -O $WORK_DIR/Cargo.toml
wget https://raw.githubusercontent.com/fcrre26/monitor/refs/heads/main/solana_pump_monitor2.sh -O $WORK_DIR/solana_pump_monitor2.sh

# 设置权限
chmod +x $WORK_DIR/solana_pump_monitor2.sh
chmod +x $INSTALL_DIR/bin/solana-monitor
chmod 600 $INSTALL_DIR/config/config.json
chown -R $USER:$USER $INSTALL_DIR

# 创建配置
echo '{"addresses":[]}' > $INSTALL_DIR/config/watch_addresses.json

# 配置DNS
echo "配置DNS..."
echo "nameserver 8.8.8.8" >> /etc/resolv.conf
echo "nameserver 1.1.1.1" >> /etc/resolv.conf

# 编译项目
echo "编译项目..."
cd $WORK_DIR
cargo build --release

# 复制编译结果
cp target/release/solana-monitor $INSTALL_DIR/bin/

# 设置环境变量
if ! grep -q "SSL_CERT_FILE" ~/.bashrc; then
    echo 'export SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt' >> ~/.bashrc
    source ~/.bashrc
fi

# 添加配置文件
cat > $HOME/.solana_pump/config/config.json << EOF
{
    "api_keys": [],
    "serverchan": {
        "keys": []
    },
    "wcf": {
        "groups": []
    },
    "proxy": {
        "enabled": false,
        "ip": "",
        "port": 0,
        "username": "",
        "password": ""
    },
    "rpc_nodes": {}
}
EOF

# 添加系统服务
sudo tee /etc/systemd/system/solana-monitor.service << EOF
[Unit]
Description=Solana Token Monitor
After=network.target

[Service]
Type=simple
User=$USER
Environment=SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
WorkingDirectory=$HOME/.solana_pump
ExecStart=$HOME/.solana_pump/bin/solana-monitor
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable solana-monitor

# 配置DNS
echo "配置DNS..."
echo "nameserver 8.8.8.8" >> /etc/resolv.conf
echo "nameserver 1.1.1.1" >> /etc/resolv.conf


echo "安装完成！"
echo -e "\n使用说明:"
echo "1. 编辑配置文件: nano ~/.solana_pump/config/config.json"
echo "2. 添加 API keys 和其他配置"
echo -e "\n服务管理:"
echo "启动: sudo systemctl start solana-monitor"
echo "停止: sudo systemctl stop solana-monitor"
echo "状态: sudo systemctl status solana-monitor"
echo "日志: journalctl -u solana-monitor -f"
echo "安装完成！"
echo "开始运行监控程序..."

# 运行脚本
./solana_pump_monitor2.sh
