import subprocess
import time
import sys

def get_total_rss(pid):
    """PID とその子プロセスすべての RSS 合計を取得する"""
    try:
        # 子プロセスの PID リストを取得
        output = subprocess.check_output(['pgrep', '-P', str(pid)], stderr=subprocess.DEVNULL)
        child_pids = output.decode().splitlines()
    except subprocess.CalledProcessError:
        child_pids = []
    
    total_rss = 0
    # 親プロセスの RSS
    try:
        output = subprocess.check_output(['ps', '-p', str(pid), '-o', 'rss', '--no-headers'], stderr=subprocess.DEVNULL)
        total_rss += int(output.decode().strip())
    except (subprocess.CalledProcessError, ValueError):
        pass

    # 子プロセスの RSS (再帰的に取得)
    for cpid in child_pids:
        total_rss += get_total_rss(cpid)
        
    return total_rss

if len(sys.argv) < 2:
    print("Usage: python3 mem_rss.py <command> [args...]")
    sys.exit(1)

cmd = sys.argv[1:]
max_rss = 0
start_time = time.time()

try:
    p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    
    while p.poll() is None:
        current_rss = get_total_rss(p.pid)
        if current_rss > max_rss:
            max_rss = current_rss
        time.sleep(0.05) # 50ms 間隔でサンプリング
    
    # 終了直後の最終チェック
    max_rss = max(max_rss, get_total_rss(p.pid))

except Exception as e:
    print(f"Error: {e}")
    sys.exit(1)

duration = time.time() - start_time
print(f"Max RSS: {max_rss / 1024:.2f} MB")
print(f"Duration: {duration:.2f} s")
