
# function watch_terraform() {
#   $(terraform output -raw monitoring_command) | \
#     sed -E 's/.*startup-script: (info: |error: |warning: )?(.*)$/\1\2/g' | \
#     awk '{
#       # Define colors
#       green="\033[32m"
#       red="\033[31m"
#       yellow="\033[33m"
#       blue="\033[34m"
#       cyan="\033[36m"
#       purple="\033[35m"
#       white="\033[37m"
#       bold="\033[1m"
#       reset="\033[0m"

#       # Process different line types
#       if ($0 ~ /[[:space:]]*[Cc]ompiling/) {
#         sub(/[[:space:]]*[Cc]ompiling/, "   " green "Compiling" reset)
#         print $0
#         next
#       }
#       else if ($0 ~ /[[:space:]]*[Dd]ownloaded/) {
#         sub(/[[:space:]]*[Dd]ownloaded/, "  " blue "Downloaded" reset)
#         print $0
#         next
#       }
#       else if ($0 ~ /[[:space:]]*[Dd]ownloading/) {
#         sub(/[[:space:]]*[Dd]ownloading/, "  " blue "Downloading" reset)
#         print $0
#         next
#       }
#       else if ($0 ~ /[[:space:]]*[Ss]etting [Uu]p/) {
#         sub(/[[:space:]]*[Ss]etting [Uu]p/, "  " blue "Setting up" reset)
#         print $0
#         next
#       }
#       else if ($0 ~ /^info:/ || $0 ~ /\[INFO\]/) {
#         if ($0 ~ /^info:/) {
#           sub(/^info:/, blue bold "info:" reset)
#         } else {
#           sub(/\[INFO\]/, blue bold "[INFO]" reset)
#         }
#         print green $0 reset
#       }
#       else if ($0 ~ /^error:/ || $0 ~ /\[ERROR\]/) {
#         print red bold $0 reset
#       }
#       else if ($0 ~ /^warning:/ || $0 ~ /\[WARN\]/) {
#         print yellow bold $0 reset
#       }
#       else if ($0 ~ /^running/) {
#         print cyan bold $0 reset
#       }
#       else if ($0 ~ /^cloning/) {
#         print blue bold $0 reset
#       }
#       else {
#         print white $0 reset
#       }
#     }'
# }


function watch_node() {
  local node_number=$1
  if [ -z "$node_number" ]; then
    echo "Usage: watch_node <node_number> (1, 2, or 3)"
    return 1
  fi

  # Get the specific monitoring command for the requested node (array is 0-based)
  local index=$((node_number - 1))
  local command=$(terraform output -json monitoring_commands | jq -r ".[$index]")

  $command | \
    sed -E 's/.*startup-script: (info: |error: |warning: )?(.*)$/\1\2/g' | \
    awk '{
      # Define colors
      green="\033[32m"
      red="\033[31m"
      yellow="\033[33m"
      blue="\033[34m"
      cyan="\033[36m"
      purple="\033[35m"
      white="\033[37m"
      bold="\033[1m"
      reset="\033[0m"

      # Process different line types
      if ($0 ~ /[[:space:]]*[Cc]ompiling/) {
        sub(/[[:space:]]*[Cc]ompiling/, "   " green "Compiling" reset)
        print $0
        next
      }
      else if ($0 ~ /[[:space:]]*[Dd]ownloaded/) {
        sub(/[[:space:]]*[Dd]ownloaded/, "  " blue "Downloaded" reset)
        print $0
        next
      }
      else if ($0 ~ /^info:/ || $0 ~ /\[INFO\]/) {
        if ($0 ~ /^info:/) {
          sub(/^info:/, green bold "info:" reset)
        } else {
          sub(/\[INFO\]/, green bold "[INFO]" reset)
        }
        print green $0 reset
      }
      else if ($0 ~ /^error:/ || $0 ~ /\[ERROR\]/) {
        print red bold $0 reset
      }
      else if ($0 ~ /^warning:/ || $0 ~ /\[WARN\]/) {
        print yellow bold $0 reset
      }
      else if ($0 ~ /^running/) {
        print cyan bold $0 reset
      }
      else if ($0 ~ /^cloning/) {
        print blue bold $0 reset
      }
      else {
        print white $0 reset
      }
    }'
}

function watch_all_nodes() {
  tmux new-session -d "watch_node 1"
  tmux split-window -h "watch_node 2"
  tmux split-window -h "watch_node 3"
  tmux select-layout even-horizontal
  tmux attach
}

watch_node $1
