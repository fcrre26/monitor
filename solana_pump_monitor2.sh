#!/bin/bash

#===========================================
# 基础配置模块
#===========================================
# Solana Pump.fun智能监控系统 v4.0
# 功能：全自动监控+市值分析+多API轮询+智能RPC管理+多通道通知

CONFIG_FILE="$HOME/.solana_pump.cfg"
LOG_FILE="$HOME/pump_monitor.log"
PY_SCRIPT="$HOME/pump_monitor.py"
RPC_FILE="$HOME/.solana_pump.rpc"
PIDFILE="/tmp/solana_pump_monitor.pid"

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

            sudo $PKG_MGR install -y python3 python3-pip jq bc curl
            pip3 install requests wcferry

            # 创建环境状态文件
            mkdir -p "$(dirname "$ENV_STATE_FILE")"
            touch "$ENV_STATE_FILE"
            
            echo -e "${GREEN}✓ 依赖安装完成${RESET}"
            ;;
        2)
            echo -e "${YELLOW}>>> 跳过环境检查${RESET}"
            # 创建环境状态文件
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
        echo "2. WeChatFerry设置"
        echo "3. 测试通知"
        echo "4. 返回主菜单"
        echo -n "请选择 [1-4]: "
        read choice
        
        case $choice in
            1)
                setup_serverchan
                ;;
            2)
                setup_wcf
                ;;
            3)
                test_notification
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

# WeChatFerry设置
setup_wcf() {
    # 检查WeChatFerry是否已安装
    if ! python3 -c "import wcferry" 2>/dev/null; then
        echo -e "${YELLOW}>>> 正在安装WeChatFerry...${RESET}"
        pip3 install wcferry
        
        echo -e "${YELLOW}>>> 是否需要安装微信Hook工具？(y/N)：${RESET}"
        read -n 1 install_hook
        echo
        if [[ $install_hook =~ ^[Yy]$ ]]; then
            python3 -m wcferry.run
        fi
    fi
    
    while true; do
        echo -e "\n${YELLOW}>>> WeChatFerry设置${RESET}"
        echo "1. 配置目标群组"
        echo "2. 删除群组配置"
        echo "3. 查看当前配置"
        echo "4. 重启WeChatFerry"
        echo "5. 返回上级菜单"
        echo -n "请选择 [1-5]: "
        read choice
        
        case $choice in
            1)
                python3 - <<EOF
import json
from wcferry import Wcf

try:
    wcf = Wcf()
    print("\n${YELLOW}>>> 正在获取群组列表...${RESET}")
    groups = wcf.get_rooms()
    
    print("\n可用的群组：")
    for i, group in enumerate(groups, 1):
        print(f"{i}. {group['name']} ({group['wxid']})")
    
    selected = input("\n请输入要添加的群组编号（多个用逗号分隔）：")
    selected_ids = [int(x.strip()) for x in selected.split(",")]
    
    with open("$CONFIG_FILE", 'r') as f:
        config = json.load(f)
    
    for idx in selected_ids:
        if 1 <= idx <= len(groups):
            group = groups[idx-1]
            if not any(g['wxid'] == group['wxid'] for g in config['wcf']['groups']):
                config['wcf']['groups'].append({
                    'wxid': group['wxid'],
                    'name': group['name']
                })
    
    with open("$CONFIG_FILE", 'w') as f:
        json.dump(config, f, indent=4)
    
    print("\n${GREEN}✓ 群组配置已更新${RESET}")
except Exception as e:
    print(f"\n${RED}配置失败: {e}${RESET}")
EOF
                ;;
            2)
                config=$(cat $CONFIG_FILE)
                groups=$(echo $config | jq -r '.wcf.groups[]')
                if [ ! -z "$groups" ]; then
                    echo -e "\n当前群组列表："
                    i=1
                    while read -r group; do
                        name=$(echo $group | jq -r '.name')
                        wxid=$(echo $group | jq -r '.wxid')
                        echo "$i. $name ($wxid)"
                        i=$((i+1))
                    done <<< "$groups"
                    
                    echo -e "\n${YELLOW}>>> 请输入要删除的群组编号：${RESET}"
                    read num
                    if [[ $num =~ ^[0-9]+$ ]]; then
                        config=$(echo $config | jq "del(.wcf.groups[$(($num-1))])")
                        echo $config > $CONFIG_FILE
                        echo -e "${GREEN}✓ 群组已删除${RESET}"
                    else
                        echo -e "${RED}无效的编号${RESET}"
                    fi
                else
                    echo -e "${YELLOW}没有已配置的群组${RESET}"
                fi
                ;;
            3)
                config=$(cat $CONFIG_FILE)
                groups=$(echo $config | jq -r '.wcf.groups[]')
                if [ ! -z "$groups" ]; then
                    echo -e "\n当前群组列表："
                    i=1
                    while read -r group; do
                        name=$(echo $group | jq -r '.name')
                        wxid=$(echo $group | jq -r '.wxid')
                        echo "$i. $name ($wxid)"
                        i=$((i+1))
                    done <<< "$groups"
                else
                    echo -e "${YELLOW}没有已配置的群组${RESET}"
                fi
                ;;
            4)
                python3 -c "
from wcferry import Wcf
try:
    wcf = Wcf()
    wcf.cleanup()
    print('${GREEN}✓ WeChatFerry已重启${RESET}')
except Exception as e:
    print(f'${RED}重启失败: {e}${RESET}')
"
                ;;
            5)
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
    python3 - <<EOF
import json
import requests
from wcferry import Wcf

def send_test_notification():
    with open("$CONFIG_FILE", 'r') as f:
        config = json.load(f)
    
    test_msg = """
🔔 通知测试
━━━━━━━━━━━━━━━━━━━━━━━━

这是一条测试消息，用于验证通知功能是否正常工作。

• Server酱
• WeChatFerry
"""
    
    # Server酱测试
    for key in config['serverchan']['keys']:
        try:
            resp = requests.post(
                f"https://sctapi.ftqq.com/{key}.send",
                data={"title": "通知测试", "desp": test_msg},
                timeout=5
            )
            if resp.status_code == 200:
                print(f"${GREEN}✓ Server酱推送成功 ({key[:8]}...{key[-8:]})${RESET}")
            else:
                print(f"${RED}✗ Server酱推送失败 ({key[:8]}...{key[-8:]})${RESET}")
        except Exception as e:
            print(f"${RED}✗ Server酱推送错误: {e}${RESET}")
    
    # WeChatFerry测试
    if config['wcf']['groups']:
        try:
            wcf = Wcf()
            for group in config['wcf']['groups']:
                try:
                    wcf.send_text(group['wxid'], test_msg)
                    print(f"${GREEN}✓ 微信推送成功 ({group['name']})${RESET}")
                except Exception as e:
                    print(f"${RED}✗ 微信推送失败 ({group['name']}): {e}${RESET}")
        except Exception as e:
            print(f"${RED}✗ WeChatFerry初始化失败: {e}${RESET}")

send_test_notification()
EOF
}

# 在 setup_notification 函数后添加:

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
PYTHON_RPC="$HOME/.solana_pump.rpc"

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
    
    # 确保Python RPC配置文件存在
    if [ ! -f "$PYTHON_RPC" ]; then
        echo "https://api.mainnet-beta.solana.com" > "$PYTHON_RPC"
    fi
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
        
        # 提取最佳节点并保存
        best_node=$(head -n 1 "$RPC_FILE" | cut -d"|" -f1)
        echo "$best_node" > "$PYTHON_RPC"
        
        # 保存完整节点信息
        nodes=$(awk -F"|" '{print "{\"endpoint\": \""$1"\", \"latency\": "$2"}"}' "$RPC_FILE" | jq -s '.')
        echo "$nodes" > "$RPC_DIR/full_rpc_info.json"
    else
        # 如果没有可用节点，使用默认节点
        echo "https://api.mainnet-beta.solana.com" > "$PYTHON_RPC"
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
    
    # 显示当前使用的节点
    if [ -f "$PYTHON_RPC" ]; then
        local current_node=$(cat "$PYTHON_RPC")
        echo -e "\n${GREEN}当前使用的节点:${RESET}"
        echo "$current_node"
    fi
    
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
# Python监控核心模块
#===========================================
cat > "$PY_SCRIPT" << 'EOF'
#!/usr/bin/env python3
import os
import time
import json
import logging
import requests
from datetime import datetime, timezone, timedelta
from wcferry import Wcf
from concurrent.futures import ThreadPoolExecutor

class TokenMonitor:
    def __init__(self):
        # 基础配置
        self.config_file = os.path.expanduser("~/.solana_pump.cfg")
        self.rpc_file = os.path.expanduser("~/.solana_pump.rpc")
        self.watch_dir = os.path.expanduser("~/.solana_pump")
        self.watch_file = os.path.join(self.watch_dir, "watch_addresses.json")
        
        # 创建必要的目录
        os.makedirs(self.watch_dir, exist_ok=True)
        
        # 初始化配置文件
        if not os.path.exists(self.config_file):
            default_config = {
                "api_keys": [],
                "serverchan": {"keys": []},
                "wcf": {"groups": []}
            }
            with open(self.config_file, 'w') as f:
                json.dump(default_config, f, indent=4)
            logging.info(f"创建默认配置文件: {self.config_file}")
        
        # 初始化RPC文件
        if not os.path.exists(self.rpc_file):
            with open(self.rpc_file, 'w') as f:
                f.write('https://api.mainnet-beta.solana.com')
            logging.info(f"创建默认RPC文件: {self.rpc_file}")
        
        # 初始化关注地址文件
        if not os.path.exists(self.watch_file):
            with open(self.watch_file, 'w') as f:
                json.dump({"addresses": {}}, f, indent=4)
            logging.info(f"创建关注地址文件: {self.watch_file}")
        
        # 加载配置
        try:
            with open(self.config_file) as f:
                self.config = json.load(f)
        except Exception as e:
            logging.error(f"加载配置失败: {e}")
            self.config = {
                "api_keys": [],
                "serverchan": {"keys": []},
                "wcf": {"groups": []}
            }
        
        self.api_keys = self.config.get('api_keys', [])
        self.current_key = 0
        self.request_counts = {}
        self.last_reset = {}
        self.wcf = None
        self.watch_addresses = self.load_watch_addresses()
        self.init_wcf()
        
        # 初始化API密钥计数器
        for key in self.api_keys:
            if key.strip():
                self.request_counts[key] = 0
                self.last_reset[key] = time.time()

        # 创建线程池
        self.executor = ThreadPoolExecutor(max_workers=5)
        
        # 缓存已分析的地址
        self.address_cache = {}
        self.cache_expire = 3600  # 缓存1小时过期
        
        # 成功项目阈值设置
        self.SUCCESS_MARKET_CAP = 50_000_000  # 5000万美元市值
        self.SUCCESS_HOLDERS = 1000  # 1000个持有人
        self.NEW_WALLET_DAYS = 7  # 新钱包定义：7天内

    def format_number(self, value):
        """格式化数字显示，使用K、M、B单位"""
        if value >= 1_000_000_000:
            return f"{value/1_000_000_000:.1f}B"
        elif value >= 1_000_000:
            return f"{value/1_000_000:.1f}M"
        elif value >= 1_000:
            return f"{value/1_000:.1f}K"
        return f"{value:.1f}"

    def format_price(self, price):
        """格式化价格显示"""
        if price < 0.00000001:  # 非常小的价格用科学计数法
            return f"${price:.2e}"
        elif price < 0.0001:    # 小价格显示更多小数位
            return f"${price:.8f}"
        elif price < 0.01:      # 较小价格显示6位小数
            return f"${price:.6f}"
        elif price < 1:         # 小于1的价格显示4位小数
            return f"${price:.4f}"
        else:                   # 其他情况显示2位小数
            return f"${self.format_number(price)}"

    def format_market_cap(self, market_cap):
        """格式化市值显示"""
        return f"${self.format_number(market_cap)}"
        
def get_best_rpc(self):
    """获取最佳RPC节点"""
    try:
        # 读取RPC配置文件
        with open(self.rpc_file) as f:
            data = f.read().strip()
            try:
                # 尝试解析JSON格式
                nodes = json.loads(data)
                if isinstance(nodes, list) and nodes:
                    return nodes[0]['endpoint']
            except json.JSONDecodeError:
                # 如果不是JSON格式，直接使用文本内容
                if data.startswith('https://'):
                    return data.strip()
        
        logging.warning(f"RPC配置读取失败，使用默认节点")
    except Exception as e:
        logging.error(f"获取RPC节点失败: {e}")
    
    # 使用默认节点
    return "https://api.mainnet-beta.solana.com"

    def load_watch_addresses(self):
        """加载关注地址"""
        try:
            with open(self.watch_file) as f:
                data = json.load(f)
                return data.get("addresses", {})
        except Exception as e:
            logging.error(f"加载关注地址失败: {e}")
            return {}

    def save_watch_addresses(self):
        """保存关注地址"""
        try:
            with open(self.watch_file, 'w') as f:
                json.dump({"addresses": self.watch_addresses}, f, indent=4)
            logging.info("关注地址更新成功")
        except Exception as e:
            logging.error(f"保存关注地址失败: {e}")

    def update_watch_address(self, address, info):
        """更新关注地址
        当发现一个地址创建的代币成功时（或其关联地址有成功记录），自动添加到关注列表
        """
        # 判断是否值得关注
        if (info['success_count'] >= 1 or  # 至少有1个成功项目
            (info.get('last_success') and info['last_success']['max_market_cap'] >= self.SUCCESS_MARKET_CAP)):
            
            if address not in self.watch_addresses:
                self.watch_addresses[address] = {
                    "success_count": info['success_count'],
                    "total_count": info['total_count'],
                    "last_success": info['last_success'],
                    "update_time": int(time.time()),
                    "first_seen": int(time.time()),  # 记录首次发现时间
                    "source": "auto_discover"  # 标记来源为自动发现
                }
                logging.info(f"自动添加关注地址: {address}, 成功项目: {info['success_count']}/{info['total_count']}")
            else:
                # 更新现有地址信息
                self.watch_addresses[address].update({
                    "success_count": info['success_count'],
                    "total_count": info['total_count'],
                    "last_success": info['last_success'],
                    "update_time": int(time.time())
                })
                logging.info(f"更新关注地址: {address}, 成功项目: {info['success_count']}/{info['total_count']}")
            
            self.save_watch_addresses()

    def get_next_api_key(self):
        """获取下一个可用的API密钥"""
        if not self.api_keys:
            raise Exception("没有配置API密钥")
        
        current_time = time.time()
        for _ in range(len(self.api_keys)):
            key = self.api_keys[self.current_key]
            
            # 检查是否需要重置计数器
            if current_time - self.last_reset.get(key, 0) > 3600:
                self.request_counts[key] = 0
                self.last_reset[key] = current_time
            
            # 如果当前密钥未达到限制
            if self.request_counts.get(key, 0) < 10:  # 每小时10次限制
                self.request_counts[key] = self.request_counts.get(key, 0) + 1
                return key
            
            # 切换到下一个密钥
            self.current_key = (self.current_key + 1) % len(self.api_keys)
        
        raise Exception("所有API密钥已达到限制")

    def init_wcf(self):
        """初始化WeChatFerry"""
        if self.config["wcf"]["groups"]:
            try:
                self.wcf = Wcf()
                logging.info("WeChatFerry初始化成功")
            except Exception as e:
                logging.error(f"WeChatFerry初始化失败: {e}")

    def fetch_token_info(self, mint):
        """获取代币信息"""
        try:
            headers = {"X-API-KEY": self.get_next_api_key()}
            url = f"https://public-api.birdeye.so/public/token?address={mint}"
            resp = requests.get(url, headers=headers, timeout=5)
            data = resp.json()
            
            if data.get("success"):
                token = data["data"]
                return {
                    "supply": float(token.get("supply", 0)),
                    "price": float(token.get("price", 0)),
                    "market_cap": float(token.get("marketCap", 0)),
                    "liquidity": float(token.get("liquidity", 0)),
                    "holder_count": int(token.get("holderCount", 0)),
                    "holder_concentration": float(token.get("holder_concentration", 0)),
                    "verified": bool(token.get("verified", False))
                }
        except Exception as e:
            logging.error(f"获取代币信息失败: {e}")
        
        return {
            "supply": 0,
            "price": 0,
            "market_cap": 0,
            "liquidity": 0,
            "holder_count": 0,
            "holder_concentration": 0,
            "verified": False
        }

    def analyze_creator_history(self, creator):
        """分析创建者历史记录"""
        # 检查缓存
        if creator in self.address_cache:
            cache_data = self.address_cache[creator]
            if time.time() - cache_data['timestamp'] < self.cache_expire:
                return cache_data['history']
        
        try:
            headers = {"X-API-KEY": self.get_next_api_key()}
            url = f"https://public-api.birdeye.so/public/address_nft_mints?address={creator}"
            resp = requests.get(url, headers=headers, timeout=5)
            data = resp.json()
            
            if data.get("success"):
                history = []
                for tx in data["data"]:
                    if "mint" in tx:
                        token_info = self.fetch_token_info(tx["mint"])
                        if token_info["market_cap"] > 0:  # 只记录有市值的代币
                            history.append({
                                "mint": tx["mint"],
                                "timestamp": tx.get("timestamp", int(time.time())),
                                "max_market_cap": token_info["market_cap"],  # 当前市值作为历史最高
                                "current_market_cap": token_info["market_cap"],
                                "status": "活跃" if token_info["market_cap"] > 0 else "已退出"
                            })
                
                # 缓存结果
                self.address_cache[creator] = {
                    'timestamp': time.time(),
                    'history': history
                }
                
                return history
        except Exception as e:
            logging.error(f"分析创建者历史失败: {e}")
        
        return []

    def analyze_creator_relations(self, creator):
        """分析创建者地址关联性"""
        try:
            related_addresses = set()
            high_value_relations = []
            watch_hits = []
            
            # 1. 分析转账历史
            headers = {"X-API-KEY": self.get_next_api_key()}
            url = f"https://public-api.birdeye.so/public/address_activity?address={creator}"
            resp = requests.get(url, headers=headers, timeout=5)
            data = resp.json()
            
            if data.get("success"):
                for tx in data["data"]:
                    # 记录所有交互过的地址
                    if tx.get("from") and tx["from"] != creator:
                        related_addresses.add(tx["from"])
                        # 分析这个地址的历史
                        history = self.analyze_creator_history(tx["from"])
                        if history:
                            success_count = sum(1 for t in history if self.check_project_success(t))
                            if success_count > 0:  # 如果是成功地址，自动添加到关注列表
                                self.update_watch_address(tx["from"], {
                                    "success_count": success_count,
                                    "total_count": len(history),
                                    "last_success": max(history, key=lambda x: x["max_market_cap"]),
                                    "update_time": int(time.time())
                                })
                                high_value_relations.append({
                                    "address": tx["from"],
                                    "success_count": success_count,
                                    "total_count": len(history),
                                    "history": sorted(history, key=lambda x: x["timestamp"], reverse=True)[:3]
                                })
                                
                    if tx.get("to") and tx["to"] != creator:
                        related_addresses.add(tx["to"])
                        # 同样分析to地址
                        history = self.analyze_creator_history(tx["to"])
                        if history:
                            success_count = sum(1 for t in history if self.check_project_success(t))
                            if success_count > 0:
                                self.update_watch_address(tx["to"], {
                                    "success_count": success_count,
                                    "total_count": len(history),
                                    "last_success": max(history, key=lambda x: x["max_market_cap"]),
                                    "update_time": int(time.time())
                                })
                                high_value_relations.append({
                                    "address": tx["to"],
                                    "success_count": success_count,
                                    "total_count": len(history),
                                    "history": sorted(history, key=lambda x: x["timestamp"], reverse=True)[:3]
                                })
                                                    # 检查是否命中关注地址
                    if tx.get("from") in self.watch_addresses:
                        watch_hits.append({
                            "address": tx["from"],
                            "info": self.watch_addresses[tx["from"]],
                            "type": "transfer_from",
                            "amount": float(tx.get("amount", 0)),
                            "timestamp": tx.get("timestamp", int(time.time()))
                        })
                    if tx.get("to") in self.watch_addresses:
                        watch_hits.append({
                            "address": tx["to"],
                            "info": self.watch_addresses[tx["to"]],
                            "type": "transfer_to",
                            "amount": float(tx.get("amount", 0)),
                            "timestamp": tx.get("timestamp", int(time.time()))
                        })

            # 2. 如果是新钱包且没有发现任何有价值关联方，返回 None
            wallet_age = self.calculate_wallet_age(creator)
            is_new_wallet = wallet_age < self.NEW_WALLET_DAYS
            if is_new_wallet and not high_value_relations:
                logging.info(f"跳过无价值新钱包: {creator}")
                return None

            return {
                "wallet_age": wallet_age,
                "is_new_wallet": is_new_wallet,
                "related_addresses": list(related_addresses),
                "high_value_relations": high_value_relations,
                "watch_hits": watch_hits,
                "risk_score": self.calculate_risk_score(wallet_age, len(related_addresses), high_value_relations)
            }
        except Exception as e:
            logging.error(f"分析地址关联性失败: {e}")
            return None

    def calculate_wallet_age(self, address):
        """计算钱包年龄（天）"""
        try:
            headers = {"X-API-KEY": self.get_next_api_key()}
            url = f"https://public-api.birdeye.so/public/address_info?address={address}"
            resp = requests.get(url, headers=headers, timeout=5)
            data = resp.json()
            
            if data.get("success"):
                first_tx_time = data["data"].get("first_tx_time", time.time())
                return (time.time() - first_tx_time) / 86400  # 转换为天数
        except Exception as e:
            logging.error(f"计算钱包年龄失败: {e}")
        
        return 0

    def check_project_success(self, token_info):
        """检查项目是否成功"""
        return (token_info["max_market_cap"] >= self.SUCCESS_MARKET_CAP or 
                token_info.get("holder_count", 0) >= self.SUCCESS_HOLDERS)

    def calculate_risk_score(self, wallet_age, related_count, high_value_relations):
        """计算风险分数"""
        score = 0
        
        # 1. 钱包年龄评分 (0-30分)
        if wallet_age < 1:  # 小于1天
            score += 30
        elif wallet_age < 7:  # 小于7天
            score += 20
        elif wallet_age < 30:  # 小于30天
            score += 10
        
        # 2. 关联地址评分 (0-30分)
        if related_count < 5:
            score += 30
        elif related_count < 20:
            score += 20
        elif related_count < 50:
            score += 10
        
        # 3. 高价值关联评分 (0-40分)
        success_relations = len(high_value_relations)
        if success_relations == 0:
            score += 40
        elif success_relations < 2:
            score += 30
        elif success_relations < 5:
            score += 20
        else:
            score += 10
        
        return min(score, 100)  # 最高100分

    def format_alert_message(self, data):
        """格式化警报消息"""
        creator = data["creator"]
        mint = data["mint"]
        token_info = data["token_info"]
        history = data["history"]
        relations = data["relations"]
                msg = f"""
🚨 新代币创建监控 (UTC+8)
━━━━━━━━━━━━━━━━━━━━━━━━

📋 基本信息:
• 代币地址: {mint}
• 创建者: {creator}
• 钱包状态: {'🆕 新钱包' if relations['is_new_wallet'] else '📅 老钱包'}
• 钱包年龄: {relations['wallet_age']:.1f} 天

💰 代币数据:
• 初始市值: {self.format_market_cap(token_info['market_cap'])}
• 代币供应量: {self.format_number(token_info['supply'])}
• 单价: {self.format_price(token_info['price'])}
• 流动性: {token_info['liquidity']:.2f} SOL
• 持有人数: {token_info['holder_count']}
• 前10持有人占比: {token_info['holder_concentration']:.1f}%"""

        # 添加关注地址信息
        if creator in self.watch_addresses:
            info = self.watch_addresses[creator]
            msg += f"""

⭐ 重点关注地址！
• 成功项目: {info['success_count']}/{info['total_count']}
• 上次成功: {datetime.fromtimestamp(info['last_success']['timestamp']).strftime('%Y-%m-%d')}
• 最高市值: {self.format_market_cap(info['last_success']['max_market_cap'])}"""

        # 添加风险评分
        risk_level = "高" if relations['risk_score'] >= 70 else "中" if relations['risk_score'] >= 40 else "低"
        msg += f"""

🎯 风险评估:
• 综合风险评分: {relations['risk_score']}/100
• 风险等级: {risk_level}
• 关联地址数: {len(relations['related_addresses'])}"""

        # 添加高价值关联信息
        if relations['high_value_relations']:
            msg += "\n\n💎 发现高价值关联方:"
            for relation in relations['high_value_relations'][:3]:  # 只显示前3个
                msg += f"""
• 地址: {relation['address']}
  - 成功项目数: {relation['success_count']}/{relation['total_count']}"""
                for token in relation['history']:
                    msg += f"""
  - {token['mint']}
    创建时间: {datetime.fromtimestamp(token['timestamp']).strftime('%Y-%m-%d %H:%M:%S')}
    最高市值: {self.format_market_cap(token['max_market_cap'])}
    当前市值: {self.format_market_cap(token['current_market_cap'])}"""

        # 添加关联的关注地址信息
        if relations['watch_hits']:
            msg += "\n\n⚠️ 发现关联的关注地址:"
            for hit in relations['watch_hits']:
                timestamp = datetime.fromtimestamp(hit["timestamp"])
                msg += f"""
• {hit['address']}
  - 成功项目: {hit['info']['success_count']}/{hit['info']['total_count']}
  - 关联类型: {hit['type']}
  - 交易金额: {hit['amount']:.2f} SOL
  - 交易时间: {timestamp.strftime('%Y-%m-%d %H:%M:%S')}"""

        # 添加创建者历史记录
        if history:
            active_tokens = sum(1 for t in history if t["status"] == "活跃")
            success_rate = len([t for t in history if self.check_project_success(t)]) / len(history) if history else 0
            msg += f"""

📜 创建者历史:
• 历史代币数: {len(history)}
• 当前活跃: {active_tokens}
• 成功率: {success_rate:.1%}

最近代币记录:"""
            for token in sorted(history, key=lambda x: x["timestamp"], reverse=True)[:3]:
                timestamp = datetime.fromtimestamp(token["timestamp"])
                msg += f"""
• {token['mint']}
  - 创建时间: {timestamp.strftime('%Y-%m-%d %H:%M:%S')}
  - 最高市值: {self.format_market_cap(token['max_market_cap'])}
  - 当前市值: {self.format_market_cap(token['current_market_cap'])}
  - 当前状态: {token['status']}"""
          # 添加投资建议
        msg += "\n\n💡 投资建议:"
        if relations['is_new_wallet'] and relations['high_value_relations']:
            msg += "\n• ⚠️ 新钱包，但发现优质关联方"
        if relations['high_value_relations']:
            msg += "\n• 🌟 发现高价值关联方，可能是成功团队新项目"
        if history and success_rate > 0.5:
            msg += "\n• ✅ 创建者历史表现良好"
        if relations['risk_score'] >= 70:
            msg += "\n• ❗ 高风险项目，建议谨慎"
        
        # 添加快速链接
        msg += f"""

🔗 快速链接:
• Birdeye: https://birdeye.so/token/{mint}
• Solscan: https://solscan.io/token/{mint}
• 创建者: https://solscan.io/account/{creator}

⏰ 发现时间: {datetime.now(tz=timezone(timedelta(hours=8))).strftime('%Y-%m-%d %H:%M:%S')} (UTC+8)
"""
        return msg

    def send_notification(self, msg):
        """发送通知"""
        # Server酱推送
        for key in self.config["serverchan"]["keys"]:
            try:
                requests.post(
                    f"https://sctapi.ftqq.com/{key}.send",
                    data={"title": "Solana新代币提醒", "desp": msg},
                    timeout=5
                )
            except Exception as e:
                logging.error(f"Server酱推送失败 ({key[:8]}...{key[-8:]}): {e}")
        
        # WeChatFerry推送
        if self.wcf and self.config["wcf"]["groups"]:
            for group in self.config["wcf"]["groups"]:
                try:
                    self.wcf.send_text(group["wxid"], msg)
                except Exception as e:
                    logging.error(f"WeChatFerry推送失败 ({group['name']}): {e}")

    def monitor(self):
        """主监控函数"""
        logging.info("监控启动...")
        last_slot = 0
        PUMP_PROGRAM = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ35MKDfgCcMKJ"
        
        while True:
            try:
                rpc = self.get_best_rpc()
                current_slot = requests.post(
                    rpc,
                    json={"jsonrpc":"2.0","id":1,"method":"getSlot"},
                    timeout=3
                ).json()["result"]
                
                if last_slot == 0:
                    last_slot = current_slot - 10
                
                for slot in range(last_slot + 1, current_slot + 1):
                    try:
                        block = requests.post(rpc, json={
                            "jsonrpc":"2.0",
                            "id":1,
                            "method":"getBlock",
                            "params":[slot, {"encoding":"json","transactionDetails":"full"}]
                        }, timeout=5).json().get("result")
                        
                        if block and "transactions" in block:
                            for tx in block["transactions"]:
                                if PUMP_PROGRAM in tx["transaction"]["message"]["accountKeys"]:
                                    accounts = tx["transaction"]["message"]["accountKeys"]
                                    creator = accounts[0]
                                    mint = accounts[4]
                                    
                                    # 分析关联性
                                    relations = self.analyze_creator_relations(creator)
                                    
                                    # 如果是新钱包且没有有价值关联方，跳过这个通知
                                    if relations is None:
                                        continue
                                    
                                    token_info = self.fetch_token_info(mint)
                                    history = self.analyze_creator_history(creator)
                                    
                                    # 如果这个创建者有成功记录，添加到关注列表
                                    if history:
                                        success_count = sum(1 for t in history if self.check_project_success(t))
                                        if success_count > 0:
                                            self.update_watch_address(creator, {
                                                "success_count": success_count,
                                                "total_count": len(history),
                                                "last_success": max(history, key=lambda x: x["max_market_cap"]),
                                                "update_time": int(time.time())
                                            })
                                    
                                    alert_data = {
                                        "creator": creator,
                                        "mint": mint,
                                        "token_info": token_info,
                                        "history": history,
                                        "relations": relations
                                    }
                                    
                                    alert_msg = self.format_alert_message(alert_data)
                                    logging.info("\n" + alert_msg)
                                    self.send_notification(alert_msg)
                    
                    except Exception as e:
                        logging.error(f"处理区块 {slot} 失败: {e}")
                        continue
                    
                    last_slot = slot
                    time.sleep(0.1)
                
                time.sleep(1)
            
            except Exception as e:
                logging.error(f"监控循环错误: {e}")
                time.sleep(10)

if __name__ == "__main__":
    logging.basicConfig(
        level=logging.INFO,
        format='%(asctime)s - %(levelname)s - %(message)s',
        handlers=[
            logging.FileHandler('monitor.log'),
            logging.StreamHandler()
        ]
    )
    monitor = TokenMonitor()
    monitor.monitor()

EOF

chmod +x "$PY_SCRIPT"
echo -e "${GREEN}✓ 监控脚本已生成${RESET}"
#===========================================
# 主程序和菜单模块
#===========================================

# 生成Python监控脚本
generate_python_script() {
    echo -e "${YELLOW}>>> 生成监控脚本...${RESET}"
cat > "$PY_SCRIPT" << 'EOF'
#!/usr/bin/env python3
import os
import sys
import time
import json
import logging
import requests
import urllib3
from datetime import datetime, timezone, timedelta
from concurrent.futures import ThreadPoolExecutor
from wcferry import Wcf

# 禁用SSL警告
urllib3.disable_warnings()

class TokenMonitor:
    def __init__(self):
        self.config_file = os.path.expanduser("~/.solana_pump.cfg")
        self.rpc_file = os.path.expanduser("~/.solana_pump.rpc")
        self.watch_file = os.path.expanduser("~/.solana_pump/watch_addresses.json")
        self.config = self.load_config()
        self.api_keys = self.config.get('api_keys', [])
        self.current_key = 0
        self.request_counts = {}
        self.last_reset = {}
        self.wcf = None
        self.watch_addresses = self.load_watch_addresses()
        self.init_wcf()
        
        # 初始化API密钥计数器
        for key in self.api_keys:
            if key.strip():
                self.request_counts[key] = 0
                self.last_reset[key] = time.time()

        # 创建线程池
        self.executor = ThreadPoolExecutor(max_workers=5)
        
        # 缓存已分析的地址
        self.address_cache = {}
        self.cache_expire = 3600  # 缓存1小时过期

    def load_config(self):
        try:
            with open(self.config_file) as f:
                return json.load(f)
        except Exception as e:
            logging.error(f"加载配置失败: {e}")
            return {"api_keys": [], "serverchan": {"keys": []}, "wcf": {"groups": []}}

    def load_watch_addresses(self):
        try:
            with open(self.watch_file) as f:
                data = json.load(f)
                return {addr['address']: addr['note'] for addr in data.get('addresses', [])}
        except Exception as e:
            logging.error(f"加载关注地址失败: {e}")
            return {}

    def init_wcf(self):
        if self.config['wcf']['groups']:
            try:
                self.wcf = Wcf()
                logging.info("WeChatFerry初始化成功")
            except Exception as e:
                logging.error(f"WeChatFerry初始化失败: {e}")
                self.wcf = None

    def get_next_api_key(self):
        """获取下一个可用的API密钥"""
        now = time.time()
        for key in self.api_keys:
            if not key.strip():
                continue
                
            if now - self.last_reset[key] >= 60:
                self.request_counts[key] = 0
                self.last_reset[key] = now
            
            if self.request_counts[key] < 100:
                self.request_counts[key] += 1
                return key
        
        raise Exception("所有API密钥已达到限制")

    def get_best_rpc(self):
        """获取最佳RPC节点"""
        try:
            with open(self.rpc_file) as f:
                nodes = [json.loads(line) for line in f]
                if not nodes:
                    raise Exception("没有可用的RPC节点")
                return nodes[0]['endpoint']
        except Exception as e:
            logging.error(f"获取RPC节点失败: {e}")
            return "https://api.mainnet-beta.solana.com"

    def fetch_token_info(self, mint):
        """获取代币详细信息"""
        try:
            headers = {"X-API-KEY": self.get_next_api_key()}
            
            # 获取基本信息
            url = f"https://public-api.birdeye.so/public/token_metadata?address={mint}"
            resp = requests.get(url, headers=headers, timeout=5)
            data = resp.json()
            
            if data.get("success"):
                token_data = data["data"]
                
                # 获取持有人信息
                holders_url = f"https://public-api.birdeye.so/public/token_holders?address={mint}"
                holders_resp = requests.get(holders_url, headers=headers, timeout=5)
                holders_data = holders_resp.json().get("data", [])
                
                # 计算持有人集中度
                total_supply = float(token_data.get("supply", 0))
                holder_concentration = 0
                if holders_data and total_supply > 0:
                    top_10_holdings = sum(float(h.get("amount", 0)) for h in holders_data[:10])
                    holder_concentration = (top_10_holdings / total_supply) * 100
                
                return {
                    "name": token_data.get("name", "Unknown"),
                    "symbol": token_data.get("symbol", "Unknown"),
                    "price": float(token_data.get("price", 0)),
                    "supply": float(token_data.get("supply", 0)),
                    "market_cap": float(token_data.get("price", 0)) * float(token_data.get("supply", 0)),
                    "liquidity": float(token_data.get("liquidity", 0)),
                    "holder_count": len(holders_data),
                    "holder_concentration": holder_concentration,
                    "verified": token_data.get("verified", False)
                }
        except Exception as e:
            logging.error(f"获取代币信息失败: {e}")
        
        return {
            "name": "Unknown",
            "symbol": "Unknown",
            "price": 0,
            "supply": 0,
            "market_cap": 0,
            "liquidity": 0,
            "holder_count": 0,
            "holder_concentration": 0,
            "verified": False
        }

    def analyze_creator_history(self, creator):
        """分析创建者历史记录"""
        # 检查缓存
        if creator in self.address_cache:
            cache_data = self.address_cache[creator]
            if time.time() - cache_data['timestamp'] < self.cache_expire:
                return cache_data['history']
        
        try:
            headers = {"X-API-KEY": self.get_next_api_key()}
            url = f"https://public-api.birdeye.so/public/address_nft_mints?address={creator}"
            resp = requests.get(url, headers=headers, timeout=5)
            data = resp.json()
            
            if data.get("success"):
                history = []
                for tx in data["data"]:
                    if "mint" in tx:
                        token_info = self.fetch_token_info(tx["mint"])
                        
                        # 获取历史最高市值
                        max_market_cap = 0
                        try:
                            history_url = f"https://public-api.birdeye.so/public/token_price_history?address={tx['mint']}"
                            history_resp = requests.get(history_url, headers=headers, timeout=5)
                            if history_resp.status_code == 200:
                                price_history = history_resp.json().get("data", [])
                                if price_history:
                                    max_price = max(float(p.get("value", 0)) for p in price_history)
                                    max_market_cap = max_price * token_info["supply"]
                        except:
                            pass

                        history.append({
                            "mint": tx["mint"],
                            "timestamp": tx["timestamp"],
                            "current_market_cap": token_info["market_cap"],
                            "max_market_cap": max_market_cap,
                            "liquidity": token_info["liquidity"],
                            "holder_count": token_info["holder_count"],
                            "holder_concentration": token_info["holder_concentration"],
                            "status": "活跃" if token_info["market_cap"] > 0 else "已退出"
                        })

                # 缓存结果
                self.address_cache[creator] = {
                    'timestamp': time.time(),
                    'history': history
                }
                return history
        except Exception as e:
            logging.error(f"获取创建者历史失败: {e}")
        
        return []

    def analyze_creator_relations(self, creator):
        """分析创建者地址关联性"""
        try:
            related_addresses = set()
            relations = []
            watch_hits = []
            high_value_relations = []
            
            # 1. 分析转账历史
            headers = {"X-API-KEY": self.get_next_api_key()}
            url = f"https://public-api.birdeye.so/public/address_activity?address={creator}"
            resp = requests.get(url, headers=headers, timeout=5)
            data = resp.json()
            
            if data.get("success"):
                # 记录地址首次交易时间
                first_tx_time = float('inf')
                for tx in data["data"]:
                    first_tx_time = min(first_tx_time, tx.get("timestamp", float('inf')))
                    
                    # 记录所有交互过的地址
                    if tx.get("from") and tx["from"] != creator:
                        related_addresses.add(tx["from"])
                        if tx["from"] in self.watch_addresses:
                            watch_hits.append({
                                'address': tx["from"],
                                'note': self.watch_addresses[tx["from"]],
                                'type': 'transfer_from',
                                'amount': tx.get("amount", 0),
                                'timestamp': tx["timestamp"]
                            })
                            
                    if tx.get("to") and tx["to"] != creator:
                        related_addresses.add(tx["to"])
                        if tx["to"] in self.watch_addresses:
                            watch_hits.append({
                                'address': tx["to"],
                                'note': self.watch_addresses[tx["to"]],
                                'type': 'transfer_to',
                                'amount': tx.get("amount", 0),
                                'timestamp': tx["timestamp"]
                            })
                        
                    # 特别关注大额转账
                    if tx.get("amount", 0) > 1:  # 1 SOL以上的转账
                        relations.append({
                            "address": tx["to"] if tx["from"] == creator else tx["from"],
                            "type": "transfer",
                            "amount": tx["amount"],
                            "timestamp": tx["timestamp"]
                        })
                
                # 计算钱包年龄（天）
                wallet_age = (time.time() - first_tx_time) / (24 * 3600) if first_tx_time != float('inf') else 0
            
            # 2. 深度分析关联地址
            for address in related_addresses:
                # 分析代币创建历史
                token_history = self.analyze_creator_history(address)
                if token_history:
                    # 找出高价值代币（最高市值超过1亿美元）
                    high_value_tokens = [t for t in token_history 
                                       if t["max_market_cap"] > 100_000_000]
                    
                    if high_value_tokens:
                        high_value_relations.append({
                            "address": address,
                            "tokens": high_value_tokens,
                            "total_created": len(token_history)
                        })
                    
                    relations.append({
                        "address": address,
                        "type": "token_creator",
                        "tokens": len(token_history),
                        "success_rate": sum(1 for t in token_history if t["status"] == "活跃") / len(token_history),
                        "high_value_tokens": len(high_value_tokens)
                    })
            
            # 3. 分析共同签名者
            with ThreadPoolExecutor(max_workers=3) as executor:
                futures = []
                for address in related_addresses:
                    futures.append(executor.submit(self._analyze_cosigners, 
                                                address, creator))
                
                for future in futures:
                    try:
                        result = future.result()
                        if result:
                            relations.extend(result)
                    except:
                        continue
            
            return {
                "wallet_age": wallet_age,
                "is_new_wallet": wallet_age < 7,  # 小于7天视为新钱包
                "related_addresses": list(related_addresses),
                "relations": relations,
                "watch_hits": watch_hits,
                "high_value_relations": high_value_relations,
                "risk_score": self.calculate_risk_score(relations, wallet_age)
            }
        except Exception as e:
            logging.error(f"分析地址关联性失败: {e}")
            return {
                "wallet_age": 0,
                "is_new_wallet": True,
                "related_addresses": [],
                "relations": [],
                "watch_hits": [],
                "high_value_relations": [],
                "risk_score": 0
            }

    def _analyze_cosigners(self, address, creator):
        """分析共同签名者（辅助函数）"""
        try:
            tx_url = f"https://public-api.solscan.io/account/transactions?account={address}"
            tx_resp = requests.get(tx_url, timeout=5)
            tx_data = tx_resp.json()
            
            cosigner_relations = []
            for tx in tx_data[:100]:  # 只看最近100笔交易
                if creator in tx.get("signatures", []):
                    cosigner_relations.append({
                        "address": address,
                        "type": "co_signer",
                        "tx_hash": tx["signature"],
                        "timestamp": tx["blockTime"]
                    })
            return cosigner_relations
        except:
            return []

    def calculate_risk_score(self, relations, wallet_age):
        """计算风险分数"""
        score = 0
        
        # 1. 钱包年龄评分 (0-25分)
        if wallet_age < 1:  # 小于1天
            score += 25
        elif wallet_age < 7:  # 小于7天
            score += 15
        elif wallet_age < 30:  # 小于30天
            score += 5
        
        # 2. 关联地址评分 (0-25分)
        unique_addresses = len(set(r["address"] for r in relations))
        if unique_addresses > 20:
            score += 25
        elif unique_addresses > 10:
            score += 15
        elif unique_addresses > 5:
            score += 5
        
        # 3. 代币创建者分析 (0-25分)
        token_creators = [r for r in relations if r["type"] == "token_creator"]
        if token_creators:
            # 计算平均成功率
            avg_success = sum(t["success_rate"] for t in token_creators) / len(token_creators)
            # 计算高价值代币数量
            high_value_count = sum(t.get("high_value_tokens", 0) for t in token_creators)
            
            if avg_success < 0.2:  # 成功率低于20%
                score += 25
            elif avg_success < 0.4:  # 成功率低于40%
                score += 15
            elif avg_success < 0.6:  # 成功率低于60%
                score += 5
            
            # 如果有高价值代币历史，降低风险分数
            if high_value_count > 0:
                score = max(0, score - 15)
        
        # 4. 交易行为评分 (0-25分)
        large_transfers = [r for r in relations if r["type"] == "transfer" and r["amount"] > 10]
        suspicious_patterns = len([t for t in large_transfers if any(
            abs(t["timestamp"] - other["timestamp"]) < 300  # 5分钟内
            for other in large_transfers
            if t != other
        )])
        
        if suspicious_patterns > 5:
            score += 25
        elif suspicious_patterns > 2:
            score += 15
        elif suspicious_patterns > 0:
            score += 5
        
        return min(score, 100)  # 最高100分

    def format_alert_message(self, data):
        """格式化警报消息"""
        creator = data["creator"]
        mint = data["mint"]
        token_info = data["token_info"]
        history = data["history"]
        relations = data["relations"]
        
        msg = f"""
🚨 新代币创建监控 (UTC+8)
━━━━━━━━━━━━━━━━━━━━━━━━

📋 基本信息:
• 代币地址: {mint}
• 创建者: {creator}
• 钱包状态: {'🆕 新钱包' if relations['is_new_wallet'] else '📅 老钱包'}
• 钱包年龄: {relations['wallet_age']:.1f} 天

💰 代币数据:
• 初始市值: ${token_info['market_cap']:,.2f}
• 代币供应量: {token_info['supply']:,.0f}
• 单价: ${token_info['price']:.8f}
• 流动性: {token_info['liquidity']:.2f} SOL
• 持有人数: {token_info['holder_count']}
• 前10持有人占比: {token_info['holder_concentration']:.1f}%"""

        # 添加关注地址信息
        if creator in self.watch_addresses:
            msg += f"\n\n⭐ 重点关注地址！\n• 备注: {self.watch_addresses[creator]}"

        # 添加风险评分
        risk_level = "高" if relations['risk_score'] >= 70 else "中" if relations['risk_score'] >= 40 else "低"
        msg += f"""

🎯 风险评估:
• 综合风险评分: {relations['risk_score']}/100
• 风险等级: {risk_level}
• 关联地址数: {len(relations['related_addresses'])}"""

        # 添加高价值关联信息
        if relations['high_value_relations']:
            msg += "\n\n💎 发现高价值关联方:"
            for relation in relations['high_value_relations'][:3]:  # 只显示前3个
                msg += f"""
• 地址: {relation['address']}
  - 创建代币总数: {relation['total_created']}
  - 高价值代币数: {len(relation['tokens'])}"""
                for token in relation['tokens'][:2]:  # 每个地址只显示前2个高价值代币
                    creation_time = datetime.fromtimestamp(token["timestamp"], tz=timezone(timedelta(hours=8)))
                    msg += f"""
  - {token['mint']}
    创建时间: {creation_time.strftime('%Y-%m-%d %H:%M:%S')}
    最高市值: ${token['max_market_cap']:,.2f}
    当前市值: ${token['current_market_cap']:,.2f}"""

        # 添加关联的关注地址信息
        if relations['watch_hits']:
            msg += "\n\n⚠️ 发现关联的关注地址:"
            for hit in relations['watch_hits']:
                timestamp = datetime.fromtimestamp(hit["timestamp"], tz=timezone(timedelta(hours=8)))
                msg += f"""
• {hit['address']}
  - 备注: {hit['note']}
  - 关联类型: {hit['type']}
  - 交易金额: {hit['amount']:.2f} SOL
  - 交易时间: {timestamp.strftime('%Y-%m-%d %H:%M:%S')}"""

        # 添加创建者历史记录
        if history:
            active_tokens = sum(1 for t in history if t["status"] == "活跃")
            success_rate = active_tokens / len(history) if history else 0
            msg += f"""

📜 创建者历史:
• 历史代币数: {len(history)}
• 当前活跃: {active_tokens}
• 成功率: {success_rate:.1%}

最近代币记录:"""
            for token in sorted(history, key=lambda x: x["timestamp"], reverse=True)[:3]:
                timestamp = datetime.fromtimestamp(token["timestamp"], tz=timezone(timedelta(hours=8)))
                msg += f"""
• {token['mint']}
  - 创建时间: {timestamp.strftime('%Y-%m-%d %H:%M:%S')}
  - 最高市值: ${token['max_market_cap']:,.2f}
  - 当前市值: ${token['current_market_cap']:,.2f}
  - 当前状态: {token['status']}"""

        # 添加投资建议
        msg += "\n\n💡 投资建议:"
        if relations['is_new_wallet']:
            msg += "\n• ⚠️ 新钱包创建，需谨慎对待"
        if relations['high_value_relations']:
            msg += "\n• 🌟 发现高价值关联方，可能是成功团队新项目"
        if success_rate > 0.5:
            msg += "\n• ✅ 创建者历史表现良好"
        if relations['risk_score'] >= 70:
            msg += "\n• ❗ 高风险项目，建议谨慎"
        
        # 添加快速链接
        msg += f"""

🔗 快速链接:
• Birdeye: https://birdeye.so/token/{mint}
• Solscan: https://solscan.io/token/{mint}
• 创建者: https://solscan.io/account/{creator}

⏰ 发现时间: {datetime.now(tz=timezone(timedelta(hours=8))).strftime('%Y-%m-%d %H:%M:%S')} (UTC+8)
"""
        return msg

    def send_notification(self, msg):
        """发送通知"""
        # Server酱推送
        for key in self.config["serverchan"]["keys"]:
            try:
                requests.post(
                    f"https://sctapi.ftqq.com/{key}.send",
                    data={"title": "Solana新代币提醒", "desp": msg},
                    timeout=5
                )
            except Exception as e:
                logging.error(f"Server酱推送失败 ({key[:8]}...{key[-8:]}): {e}")
        
        # WeChatFerry推送
        if self.wcf and self.config["wcf"]["groups"]:
            for group in self.config["wcf"]["groups"]:
                try:
                    self.wcf.send_text(group["wxid"], msg)
                except Exception as e:
                    logging.error(f"WeChatFerry推送失败 ({group['name']}): {e}")

    def monitor(self):
        """主监控函数"""
        logging.info("监控启动...")
        last_slot = 0
        PUMP_PROGRAM = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ35MKDfgCcMKJ"
        
        while True:
            try:
                rpc = self.get_best_rpc()
                current_slot = requests.post(
                    rpc,
                    json={"jsonrpc":"2.0","id":1,"method":"getSlot"},
                    timeout=3
                ).json()["result"]
                
                if last_slot == 0:
                    last_slot = current_slot - 10
                
                for slot in range(last_slot + 1, current_slot + 1):
                    try:
                        block = requests.post(rpc, json={
                            "jsonrpc":"2.0",
                            "id":1,
                            "method":"getBlock",
                            "params":[slot, {"encoding":"json","transactionDetails":"full"}]
                        }, timeout=5).json().get("result")
                        
                        if block and "transactions" in block:
                            for tx in block["transactions"]:
                                if PUMP_PROGRAM in tx["transaction"]["message"]["accountKeys"]:
                                    accounts = tx["transaction"]["message"]["accountKeys"]
                                    creator = accounts[0]
                                    mint = accounts[4]
                                    
                                    token_info = self.fetch_token_info(mint)
                                    history = self.analyze_creator_history(creator)
                                    relations = self.analyze_creator_relations(creator)
                                    
                                    alert_data = {
                                        "creator": creator,
                                        "mint": mint,
                                        "token_info": token_info,
                                        "history": history,
                                        "relations": relations
                                    }
                                    
                                    alert_msg = self.format_alert_message(alert_data)
                                    logging.info("\n" + alert_msg)
                                    self.send_notification(alert_msg)
                    
                    except Exception as e:
                        logging.error(f"处理区块 {slot} 失败: {e}")
                        continue
                    
                    last_slot = slot
                    time.sleep(0.1)
                
                time.sleep(1)
            
            except Exception as e:
                logging.error(f"监控循环错误: {e}")
                time.sleep(10)

if __name__ == "__main__":
    logging.basicConfig(
        level=logging.INFO,
        format='%(asctime)s - %(levelname)s - %(message)s',
        handlers=[
            logging.FileHandler('monitor.log'),
            logging.StreamHandler()
        ]
    )
    monitor = TokenMonitor()
    monitor.monitor()
EOF

    chmod +x "$PY_SCRIPT"
    echo -e "${GREEN}✓ 监控脚本已生成${RESET}"
}

# 前后台控制
toggle_foreground() {
    if [ -f "$PIDFILE" ]; then
        pid=$(cat "$PIDFILE")
        if kill -0 "$pid" 2>/dev/null; then
            echo -e "${GREEN}>>> 切换到前台显示...${RESET}"
            tail -f "$LOG_FILE"
        else
            echo -e "${RED}>>> 监控进程未运行${RESET}"
        fi
    else
        echo -e "${RED}>>> 监控进程未运行${RESET}"
    fi
}

# 启动监控
start_monitor() {
        # 检查RPC配置
    if [ ! -f "$PYTHON_RPC" ] || [ ! -s "$PYTHON_RPC" ]; then
        echo -e "${YELLOW}>>> RPC配置不存在或为空，执行RPC测试...${RESET}"
        test_all_nodes
    fi
    
    # 验证RPC配置
    if [ ! -f "$PYTHON_RPC" ] || [ ! -s "$PYTHON_RPC" ]; then
        echo -e "${RED}>>> RPC配置失败，无法启动监控${RESET}"
        return 1
    fi
      
    if [ -f "$PIDFILE" ]; then
        pid=$(cat "$PIDFILE")
        if kill -0 "$pid" 2>/dev/null; then
            echo -e "${YELLOW}>>> 监控已在运行 (PID: $pid)${RESET}"
            echo -e "${YELLOW}>>> 是否切换到前台显示? (y/N)：${RESET}"
            read -n 1 show_log
            echo
            if [[ $show_log =~ ^[Yy]$ ]]; then
                toggle_foreground
            fi
            return
        fi
    fi
    
    generate_python_script
    echo -e "${GREEN}>>> 启动监控进程...${RESET}"
    nohup python3 $PY_SCRIPT > "$LOG_FILE" 2>&1 & 
    echo $! > "$PIDFILE"
    echo -e "${GREEN}>>> 监控已在后台启动 (PID: $!)${RESET}"
    echo -e "${GREEN}>>> 使用'3'选项可切换前台显示${RESET}"
}

# 主菜单
show_menu() {
    echo -e "\n${BLUE}Solana Pump监控系统 v4.0${RESET}"
    echo "1. 启动监控"
    echo "2. 配置API密钥"
    echo "3. 切换前台显示"
    echo "4. RPC节点管理"
    echo "5. 通知设置"
    echo "6. 关注地址管理"
    echo "7. 环境管理"    # 新增选项
    echo "8. 退出"
    echo -n "请选择 [1-8]: "
}


# 主程序入口
case $1 in
    "--daemon")
        generate_python_script
        exec python3 $PY_SCRIPT
        ;;
    "--start")
        install_dependencies
        start_monitor
        ;;
    *)
        install_dependencies
        while true; do
            show_menu
            read choice
            case $choice in
                1) start_monitor ;;
                2) init_config ;;
                3) toggle_foreground ;;
                4) manage_rpc ;;
                5) setup_notification ;;
                6) manage_watch_addresses ;;
                7) manage_environment ;;
                8) 
                    if [ -f "$PIDFILE" ]; then
                        pid=$(cat "$PIDFILE")
                        kill "$pid" 2>/dev/null
                        rm "$PIDFILE"
                    fi
                    exit 0 
                    ;;
                *) echo -e "${RED}无效选项!${RESET}" ;;
            esac
        done
        ;;
esac
