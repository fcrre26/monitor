#!/bin/bash

#===========================================
# 基础配置模块
#===========================================
# Solana Pump.fun智能监控系统 v4.0
# 功能：全自动监控+市值分析+多API轮询+智能RPC管理+多通道通知

CONFIG_FILE="$HOME/.solana_pump.cfg"
LOG_FILE="$HOME/pump_monitor.log"
RPC_FILE="$HOME/.solana_pump.rpc"
PIDFILE="/tmp/solana_pump_monitor.pid"
RUST_BIN="$HOME/.solana_pump/monitor"
RUST_SRC="monitor.rs"

# 颜色定义
RED='\033[31m'
GREEN='\033[32m'
YELLOW='\033[33m'
BLUE='\033[34m'
RESET='\033[0m'

#===========================================
# 配置管理模块
#===========================================
# 环境状态文件
ENV_STATE_FILE="$HOME/.solana_pump/env_state"

# 检查环境状态
check_env_state() {
    if [ -f "$ENV_STATE_FILE" ]; then
        return 0
    fi
    return 1
}

# API密钥配置
init_config() {
    echo -e "${YELLOW}>>> 配置API密钥 (支持多个，每行一个)${RESET}"
    echo -e "${YELLOW}>>> 输入完成后请按Ctrl+D结束${RESET}"
    api_keys=$(cat)
    
    # 创建默认配置
    config='{
        "api_keys": [],
        "serverchan": {
            "keys": []
        },
        "wcf": {
            "groups": []
        }
    }'
    
    # 添加API密钥
    for key in $api_keys; do
        if [ ! -z "$key" ]; then
            config=$(echo $config | jq --arg key "$key" '.api_keys += [$key]')
        fi
    done
    
    echo $config > $CONFIG_FILE
    chmod 600 $CONFIG_FILE
    echo -e "\n${GREEN}✓ 配置已保存到 $CONFIG_FILE${RESET}"
}

# 依赖安装
install_dependencies() {
    # 如果环境状态文件存在，直接返回
    if check_env_state; then
        return 0
    fi

    echo -e "\n${YELLOW}>>> 首次运行需要检查环境${RESET}"
    echo -e "1. 检查并安装依赖"
    echo -e "2. 跳过检查（如果确定环境已准备好）"
    echo -n "请选择 [1-2]: "
    read choice

    case $choice in
        1)
            echo -e "${YELLOW}>>> 开始安装依赖...${RESET}"
            if command -v apt &>/dev/null; then
                PKG_MGR="apt"
                sudo apt update
            elif command -v yum &>/dev/null; then
                PKG_MGR="yum"
            else
                echo -e "${RED}✗ 不支持的系统!${RESET}"
                exit 1
            fi

            sudo $PKG_MGR install -y jq bc curl
            
            # 创建环境状态文件
            mkdir -p "$(dirname "$ENV_STATE_FILE")"
            touch "$ENV_STATE_FILE"
            
            echo -e "${GREEN}✓ 依赖安装完成${RESET}"
            ;;
        2)
            echo -e "${YELLOW}>>> 跳过环境检查${RESET}"
            mkdir -p "$(dirname "$ENV_STATE_FILE")"
            touch "$ENV_STATE_FILE"
            ;;
        *)
            echo -e "${RED}无效选项!${RESET}"
            exit 1
            ;;
    esac
}

# 环境管理
manage_environment() {
    while true; do
        echo -e "\n${YELLOW}>>> 环境管理${RESET}"
        echo "1. 检查环境状态"
        echo "2. 重新安装依赖"
        echo "3. 清除环境状态"
        echo "4. 返回主菜单"
        echo -n "请选择 [1-4]: "
        read choice

        case $choice in
            1)
                if check_env_state; then
                    echo -e "${GREEN}✓ 环境已配置${RESET}"
                else
                    echo -e "${YELLOW}环境未配置${RESET}"
                fi
                ;;
            2)
                rm -f "$ENV_STATE_FILE"
                install_dependencies
                ;;
            3)
                rm -f "$ENV_STATE_FILE"
                echo -e "${GREEN}✓ 环境状态已清除${RESET}"
                ;;
            4)
                return
                ;;
            *)
                echo -e "${RED}无效选项!${RESET}"
                ;;
        esac
    done
}

#===========================================
# 通知系统模块
#===========================================
setup_notification() {
    while true; do
        echo -e "\n${YELLOW}>>> 通知设置${RESET}"
        echo "1. Server酱设置"
        echo "2. 测试通知"
        echo "3. 返回主菜单"
        echo -n "请选择 [1-3]: "
        read choice
        
        case $choice in
            1)
                setup_serverchan
                ;;
            2)
                test_notification
                ;;
            3)
                return
                ;;
            *)
                echo -e "${RED}无效选项!${RESET}"
                ;;
        esac
    done
}

# Server酱设置
setup_serverchan() {
    while true; do
        echo -e "\n${YELLOW}>>> Server酱设置${RESET}"
        echo "1. 添加Server酱密钥"
        echo "2. 删除Server酱密钥"
        echo "3. 查看当前密钥"
        echo "4. 返回上级菜单"
        echo -n "请选择 [1-4]: "
        read choice
        
        case $choice in
            1)
                echo -e "${YELLOW}>>> 请输入Server酱密钥：${RESET}"
                read -s key
                echo
                if [ ! -z "$key" ]; then
                    config=$(cat $CONFIG_FILE)
                    config=$(echo $config | jq --arg key "$key" '.serverchan.keys += [$key]')
                    echo $config > $CONFIG_FILE
                    echo -e "${GREEN}✓ Server酱密钥已添加${RESET}"
                fi
                ;;
            2)
                config=$(cat $CONFIG_FILE)
                keys=$(echo $config | jq -r '.serverchan.keys[]')
                if [ ! -z "$keys" ]; then
                    echo -e "\n当前密钥列表："
                    i=1
                    while read -r key; do
                        echo "$i. ${key:0:8}...${key: -8}"
                        i=$((i+1))
                    done <<< "$keys"
                    
                    echo -e "\n${YELLOW}>>> 请输入要删除的密钥编号：${RESET}"
                    read num
                    if [[ $num =~ ^[0-9]+$ ]]; then
                        config=$(echo $config | jq "del(.serverchan.keys[$(($num-1))])")
                        echo $config > $CONFIG_FILE
                        echo -e "${GREEN}✓ 密钥已删除${RESET}"
                    else
                        echo -e "${RED}无效的编号${RESET}"
                    fi
                else
                    echo -e "${YELLOW}没有已保存的密钥${RESET}"
                fi
                ;;
            3)
                config=$(cat $CONFIG_FILE)
                keys=$(echo $config | jq -r '.serverchan.keys[]')
                if [ ! -z "$keys" ]; then
                    echo -e "\n当前密钥列表："
                    i=1
                    while read -r key; do
                        echo "$i. ${key:0:8}...${key: -8}"
                        i=$((i+1))
                    done <<< "$keys"
                else
                    echo -e "${YELLOW}没有已保存的密钥${RESET}"
                fi
                ;;
            4)
                return
                ;;
            *)
                echo -e "${RED}无效选项!${RESET}"
                ;;
        esac
    done
}

# 测试通知
test_notification() {
    echo -e "${YELLOW}>>> 发送测试通知...${RESET}"
    config=$(cat $CONFIG_FILE)
    keys=$(echo $config | jq -r '.serverchan.keys[]')
    
    test_msg="🔔 通知测试\n━━━━━━━━━━━━━━━━━━━━━━━━\n\n这是一条测试消息，用于验证通知功能是否正常工作。"
    
    if [ ! -z "$keys" ]; then
        while read -r key; do
            response=$(curl -s "https://sctapi.ftqq.com/${key}.send" \
                          -d "title=通知测试" \
                          -d "desp=${test_msg}")
            if [[ "$response" == *"success"* ]]; then
                echo -e "${GREEN}✓ Server酱推送成功 (${key:0:8}...${key: -8})${RESET}"
            else
                echo -e "${RED}✗ Server酱推送失败 (${key:0:8}...${key: -8})${RESET}"
            fi
        done <<< "$keys"
    else
        echo -e "${YELLOW}没有配置Server酱密钥${RESET}"
    fi
}

#===========================================
# 关注地址管理模块
#===========================================
manage_watch_addresses() {
    local WATCH_DIR="$HOME/.solana_pump"
    local WATCH_FILE="$WATCH_DIR/watch_addresses.json"
    
    # 创建目录和文件（如果不存在）
    mkdir -p "$WATCH_DIR"
    if [ ! -f "$WATCH_FILE" ]; then
        echo '{"addresses":[]}' > "$WATCH_FILE"
    fi
    
    while true; do
        echo -e "\n${YELLOW}>>> 关注地址管理${RESET}"
        echo "1. 添加关注地址"
        echo "2. 删除关注地址"
        echo "3. 查看当前地址"
        echo "4. 返回主菜单"
        echo -n "请选择 [1-4]: "
        read choice
        
        case $choice in
            1)
                echo -e "\n${YELLOW}>>> 添加关注地址${RESET}"
                echo -n "请输入Solana地址: "
                read address
                echo -n "请输入备注信息: "
                read note
                
                if [ ! -z "$address" ]; then
                    # 检查地址格式
                    if [[ ! "$address" =~ ^[1-9A-HJ-NP-Za-km-z]{32,44}$ ]]; then
                        echo -e "${RED}无效的Solana地址格式${RESET}"
                        continue
                    fi
                    
                    # 添加地址
                    tmp=$(mktemp)
                    jq --arg addr "$address" --arg note "$note" \
                        '.addresses += [{"address": $addr, "note": $note}]' \
                        "$WATCH_FILE" > "$tmp" && mv "$tmp" "$WATCH_FILE"
                    
                    echo -e "${GREEN}✓ 地址已添加${RESET}"
                fi
                ;;
            2)
                addresses=$(jq -r '.addresses[] | "\(.address) (\(.note))"' "$WATCH_FILE")
                if [ ! -z "$addresses" ]; then
                    echo -e "\n当前关注地址："
                    i=1
                    while IFS= read -r line; do
                        echo "$i. $line"
                        i=$((i+1))
                    done <<< "$addresses"
                    
                    echo -e "\n${YELLOW}>>> 请输入要删除的地址编号：${RESET}"
                    read num
                    if [[ $num =~ ^[0-9]+$ ]]; then
                        tmp=$(mktemp)
                        jq "del(.addresses[$(($num-1))])" "$WATCH_FILE" > "$tmp" \
                            && mv "$tmp" "$WATCH_FILE"
                        echo -e "${GREEN}✓ 地址已删除${RESET}"
                    else
                        echo -e "${RED}无效的编号${RESET}"
                    fi
                else
                    echo -e "${YELLOW}没有已添加的关注地址${RESET}"
                fi
                ;;
            3)
                addresses=$(jq -r '.addresses[] | "\(.address) (\(.note))"' "$WATCH_FILE")
                if [ ! -z "$addresses" ]; then
                    echo -e "\n当前关注地址："
                    i=1
                    while IFS= read -r line; do
                        echo "$i. $line"
                        i=$((i+1))
                    done <<< "$addresses"
                else
                    echo -e "${YELLOW}没有已添加的关注地址${RESET}"
                fi
                ;;
            4)
                return
                ;;
            *)
                echo -e "${RED}无效选项!${RESET}"
                ;;
        esac
    done
}

#===========================================
# RPC节点处理模块
#===========================================
# 全局配置
RPC_DIR="$HOME/.solana_pump"
RPC_FILE="$RPC_DIR/rpc_list.txt"
CUSTOM_NODES="$RPC_DIR/custom_nodes.txt"

# 状态指示图标
STATUS_OK="[OK]"
STATUS_SLOW="[!!]"
STATUS_ERROR="[XX]"

# 延迟阈值(毫秒)
LATENCY_GOOD=100    # 良好延迟阈值
LATENCY_WARN=500    # 警告延迟阈值

# 默认RPC节点列表
DEFAULT_RPC_NODES=(
    "https://api.mainnet-beta.solana.com"
    "https://solana-api.projectserum.com"
    "https://rpc.ankr.com/solana"
    "https://solana-mainnet.rpc.extrnode.com"
    "https://api.mainnet.rpcpool.com"
    "https://api.metaplex.solana.com"
    "https://api.solscan.io"
    "https://solana.public-rpc.com"
)

# 初始化RPC配置
init_rpc_config() {
    mkdir -p "$RPC_DIR"
    touch "$RPC_FILE"
    touch "$CUSTOM_NODES"
}

# 测试单个RPC节点
test_rpc_node() {
    local endpoint="$1"
    local timeout=5
    
    # 构建测试请求
    local request='{
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getHealth"
    }'
    
    # 测试节点
    local start_time=$(date +%s.%N)
    local response=$(curl -s -X POST -H "Content-Type: application/json" \
                    -d "$request" \
                    --connect-timeout $timeout \
                    "$endpoint" 2>/dev/null)
    local end_time=$(date +%s.%N)
    
    # 计算延迟(ms)
    local latency=$(echo "($end_time - $start_time) * 1000" | bc)
    
    # 验证响应
    if [ ! -z "$response" ] && [[ "$response" == *"result"* ]]; then
        local status
        if (( $(echo "$latency < $LATENCY_GOOD" | bc -l) )); then
            status="$STATUS_OK"
        elif (( $(echo "$latency < $LATENCY_WARN" | bc -l) )); then
            status="$STATUS_SLOW"
        else
            status="$STATUS_ERROR"
        fi
        echo "$endpoint|$latency|$status"
        return 0
    fi
    return 1
}

# 测试所有节点
test_all_nodes() {
    local temp_file="$RPC_DIR/temp_results.txt"
    > "$temp_file"
    
    echo -e "\n${YELLOW}>>> 开始测试节点...${RESET}"
    local total=0
    local success=0
    
    # 测试默认节点
    for endpoint in "${DEFAULT_RPC_NODES[@]}"; do
        ((total++))
        echo -ne "\r测试进度: $total"
        if result=$(test_rpc_node "$endpoint"); then
            echo "$result" >> "$temp_file"
            ((success++))
        fi
    done
    
    # 测试自定义节点
    if [ -f "$CUSTOM_NODES" ]; then
        while read -r endpoint; do
            [ -z "$endpoint" ] && continue
            ((total++))
            echo -ne "\r测试进度: $total"
            if result=$(test_rpc_node "$endpoint"); then
                echo "$result" >> "$temp_file"
                ((success++))
            fi
        done < "$CUSTOM_NODES"
    fi
    
    # 按延迟排序并保存结果
    if [ -f "$temp_file" ] && [ -s "$temp_file" ]; then
        sort -t"|" -k2 -n "$temp_file" -o "$RPC_FILE"
        
        # 保存完整节点信息
        nodes=$(awk -F"|" '{print "{\"endpoint\": \""$1"\", \"latency\": "$2"}"}' "$RPC_FILE" | jq -s '.')
        echo "$nodes" > "$RPC_DIR/full_rpc_info.json"
    fi
    
    rm -f "$temp_file"
    
    echo -e "\n\n${GREEN}✓ 测试完成"
    echo "总节点数: $total"
    echo "可用节点数: $success"
    echo -e "可用率: $(( success * 100 / total ))%${RESET}"
    
    # 显示最佳节点
    if [ -f "$RPC_FILE" ] && [ -s "$RPC_FILE" ]; then
        echo -e "\n最佳节点 (延迟<${LATENCY_GOOD}ms):"
        echo "------------------------------------------------"
        head -n 5 "$RPC_FILE" | while IFS="|" read -r endpoint latency status; do
            printf "%-4s %7.1f ms  %s\n" "$status" "$latency" "$endpoint"
        done
    fi
}

# 添加自定义节点
add_custom_node() {
    echo -e "${YELLOW}>>> 添加自定义RPC节点${RESET}"
    echo -n "请输入节点地址: "
    read endpoint
    
    if [ ! -z "$endpoint" ]; then
        # 验证节点格式
        if [[ ! "$endpoint" =~ ^https?:// ]]; then
            echo -e "${RED}错误: 无效的节点地址格式，必须以 http:// 或 https:// 开头${RESET}"
            return 1
        fi
        
        # 检查是否已存在
        if grep -q "^$endpoint$" "$CUSTOM_NODES" 2>/dev/null; then
            echo -e "${YELLOW}该节点已存在${RESET}"
            return 1
        fi
        
        # 测试节点连接
        echo -e "${YELLOW}正在测试节点连接...${RESET}"
        if result=$(test_rpc_node "$endpoint"); then
            echo "$endpoint" >> "$CUSTOM_NODES"
            echo -e "${GREEN}✓ 节点已添加并测试通过${RESET}"
            test_all_nodes
        else
            echo -e "${RED}✗ 节点连接测试失败${RESET}"
            return 1
        fi
    fi
}

# 删除自定义节点
delete_custom_node() {
    if [ ! -f "$CUSTOM_NODES" ] || [ ! -s "$CUSTOM_NODES" ]; then
        echo -e "${RED}>>> 没有自定义节点${RESET}"
        return 1
    fi
    
    echo -e "\n${YELLOW}>>> 当前自定义节点：${RESET}"
    nl -w3 -s". " "$CUSTOM_NODES"
    echo -n "请输入要删除的节点编号 (输入 0 取消): "
    read num
    
    if [ "$num" = "0" ]; then
        echo -e "${YELLOW}已取消删除${RESET}"
        return 0
    fi
    
    if [[ $num =~ ^[0-9]+$ ]]; then
        local total_lines=$(wc -l < "$CUSTOM_NODES")
        if [ "$num" -le "$total_lines" ]; then
            local node_to_delete=$(sed "${num}!d" "$CUSTOM_NODES")
            sed -i "${num}d" "$CUSTOM_NODES"
            echo -e "${GREEN}✓ 已删除节点: $node_to_delete${RESET}"
            test_all_nodes
        else
            echo -e "${RED}错误: 无效的节点编号${RESET}"
            return 1
        fi
    else
        echo -e "${RED}错误: 请输入有效的数字${RESET}"
        return 1
    fi
}

# 查看当前节点
view_current_nodes() {
    echo -e "\n${YELLOW}>>> RPC节点状态：${RESET}"
    
    # 显示所有节点列表
    if [ -f "$RPC_FILE" ] && [ -s "$RPC_FILE" ]; then
        echo -e "\n所有可用节点:"
        echo -e "状态   延迟(ms)  节点地址"
        echo "------------------------------------------------"
        while IFS="|" read -r endpoint latency status; do
            printf "%-4s %7.1f ms  %s\n" "$status" "$latency" "$endpoint"
        done < "$RPC_FILE"
    else
        echo -e "${YELLOW}>>> 没有测试过的节点记录${RESET}"
    fi
    
    # 显示自定义节点
    if [ -f "$CUSTOM_NODES" ] && [ -s "$CUSTOM_NODES" ]; then
        echo -e "\n自定义节点列表:"
        nl -w3 -s". " "$CUSTOM_NODES"
    fi
}

# RPC节点管理主函数
manage_rpc() {
    # 确保配置已初始化
    init_rpc_config
    
    while true; do
        echo -e "\n${YELLOW}>>> RPC节点管理${RESET}"
        echo "1. 添加自定义节点"
        echo "2. 查看当前节点"
        echo "3. 测试节点延迟"
        echo "4. 使用默认节点"
        echo "5. 删除自定义节点"
        echo "6. 返回主菜单"
        echo -n "请选择 [1-6]: "
        read choice
        
        case $choice in
            1)
                add_custom_node
                ;;
            2)
                view_current_nodes
                ;;
            3)
                test_all_nodes
                ;;
            4)
                echo -e "${YELLOW}>>> 使用默认RPC节点...${RESET}"
                test_all_nodes
                ;;
            5)
                delete_custom_node
                ;;
            6)
                return
                ;;
            *)
                echo -e "${RED}无效选项!${RESET}"
                ;;
        esac
    done
}

#===========================================
# 环境管理模块
#===========================================
# 检查环境状态
check_env() {
    echo -e "${YELLOW}>>> 检查环境...${RESET}"
    
    # 检查必要工具
    local tools=("jq" "bc" "curl")
    local missing=()
    
    for tool in "${tools[@]}"; do
        if ! command -v $tool &>/dev/null; then
            missing+=($tool)
        fi
    done
    
    # 检查Rust环境
    if ! command -v rustc &>/dev/null || ! command -v cargo &>/dev/null; then
        echo -e "${YELLOW}>>> 未检测到Rust环境，准备安装...${RESET}"
        install_rust
    fi
    
    if [ ${#missing[@]} -ne 0 ]; then
        echo -e "${RED}缺少以下工具: ${missing[*]}${RESET}"
        echo -e "${YELLOW}>>> 准备安装缺失的工具...${RESET}"
        
        if command -v apt &>/dev/null; then
            sudo apt update
            sudo apt install -y "${missing[@]}"
        elif command -v yum &>/dev/null; then
            sudo yum install -y "${missing[@]}"
        else
            echo -e "${RED}✗ 不支持的系统!${RESET}"
            exit 1
        fi
    fi
    
    echo -e "${GREEN}✓ 环境检查完成${RESET}"
}

# 安装Rust环境
install_rust() {
    echo -e "${YELLOW}>>> 安装Rust环境...${RESET}"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    rustup default stable
    echo -e "${GREEN}✓ Rust环境安装完成${RESET}"
}

#===========================================
# RPC节点管理模块
#===========================================
# 初始化RPC配置
init_rpc() {
    mkdir -p "$(dirname "$RPC_FILE")"
    if [ ! -f "$RPC_FILE" ]; then
        echo "https://api.mainnet-beta.solana.com" > "$RPC_FILE"
    fi
}

# 测试RPC节点
test_rpc() {
    local endpoint=$(cat "$RPC_FILE")
    echo -e "${YELLOW}>>> 测试RPC节点: $endpoint${RESET}"
    
    local response=$(curl -s -X POST -H "Content-Type: application/json" \
                    -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' \
                    "$endpoint")
    
    if [[ "$response" == *"result"* ]]; then
        echo -e "${GREEN}✓ RPC节点连接正常${RESET}"
        return 0
    else
        echo -e "${RED}✗ RPC节点连接失败${RESET}"
        return 1
    fi
}

#===========================================
# Rust监控核心模块
#===========================================
# 编译Rust程序
build_monitor() {
    echo -e "${YELLOW}>>> 编译监控程序...${RESET}"
    if [ ! -f "$RUST_SRC" ]; then
        echo -e "${RED}✗ 未找到monitor.rs源文件${RESET}"
        return 1
    fi
    
    rustc "$RUST_SRC" -o "$RUST_BIN"
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}✓ 编译成功${RESET}"
        return 0
    else
        echo -e "${RED}✗ 编译失败${RESET}"
        return 1
    fi
}

# 启动监控
start_monitor() {
    if [ -f "$PIDFILE" ]; then
        pid=$(cat "$PIDFILE")
        if kill -0 "$pid" 2>/dev/null; then
            echo -e "${YELLOW}监控程序已在运行 (PID: $pid)${RESET}"
            return 1
        fi
        rm -f "$PIDFILE"
    fi
    
    if [ ! -f "$RUST_BIN" ]; then
        echo -e "${YELLOW}>>> 监控程序未编译，准备编译...${RESET}"
        build_monitor || return 1
    fi
    
    echo -e "${YELLOW}>>> 启动监控程序...${RESET}"
    nohup "$RUST_BIN" > "$LOG_FILE" 2>&1 &
    echo $! > "$PIDFILE"
    echo -e "${GREEN}✓ 监控程序已启动 (PID: $!)${RESET}"
}

# 停止监控
stop_monitor() {
    if [ -f "$PIDFILE" ]; then
        pid=$(cat "$PIDFILE")
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid"
            rm -f "$PIDFILE"
            echo -e "${GREEN}✓ 监控程序已停止${RESET}"
        else
            echo -e "${YELLOW}监控程序未在运行${RESET}"
            rm -f "$PIDFILE"
        fi
    else
        echo -e "${YELLOW}监控程序未在运行${RESET}"
    fi
}

# 查看监控状态
check_status() {
    if [ -f "$PIDFILE" ]; then
        pid=$(cat "$PIDFILE")
        if kill -0 "$pid" 2>/dev/null; then
            echo -e "${GREEN}监控程序运行中 (PID: $pid)${RESET}"
            echo -e "\n最近的日志:"
            tail -n 10 "$LOG_FILE"
        else
            echo -e "${YELLOW}监控程序未在运行${RESET}"
            rm -f "$PIDFILE"
        fi
    else
        echo -e "${YELLOW}监控程序未在运行${RESET}"
    fi
}

#===========================================
# 主程序
#===========================================
main() {
    # 初始化
    check_env
    init_rpc
    
    # 主菜单
    while true; do
        echo -e "\n${YELLOW}=== Solana Pump监控系统 v4.0 ===${RESET}"
        echo "1. 环境管理"
        echo "2. API密钥配置"
        echo "3. 通知设置"
        echo "4. RPC节点管理"
        echo "5. 关注地址管理"
        echo "6. 启动监控"
        echo "7. 停止监控"
        echo "8. 查看状态"
        echo "9. 退出"
        echo -n "请选择 [1-9]: "
        read choice
        
        case $choice in
            1) manage_environment ;;
            2) init_config ;;
            3) setup_notification ;;
            4) manage_rpc ;;
            5) manage_watch_addresses ;;
            6) start_monitor ;;
            7) stop_monitor ;;
            8) check_status ;;
            9) 
                echo -e "${GREEN}感谢使用，再见！${RESET}"
                exit 0
                ;;
            *) echo -e "${RED}无效选项!${RESET}" ;;
        esac
    done
}

# 启动主程序
main
