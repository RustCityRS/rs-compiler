#!/bin/bash

echo "Building RuneScript Compiler..."
cargo build --release

# Create installation directory
INSTALL_DIR="$HOME/.rsc"
mkdir -p "$INSTALL_DIR/bin"

# Function to safely copy the binary
copy_with_retry() {
    local source=$1
    local target=$2
    local max_retries=3
    local retry_wait=2
    local attempt=1

    while [ $attempt -le $max_retries ]; do
        echo "Installation attempt $attempt..."
        
        # If file exists, try to stop any running processes
        if [ -f "$target" ]; then
            echo "Stopping existing RSC processes..."
            pkill -f "$target" 2>/dev/null || true
            sleep 1
        fi
        
        # Try to copy
        if cp "$source" "$target" 2>/dev/null; then
            chmod +x "$target"
            return 0
        fi
        
        if [ $attempt -eq $max_retries ]; then
            echo "Error: Could not install RSC after $max_retries attempts. Please close any running instances and try again."
            return 1
        fi
        
        echo "Installation attempt $attempt failed. Retrying in $retry_wait seconds..."
        sleep $retry_wait
        attempt=$((attempt + 1))
    done
}

# Copy the binary
echo "Installing RuneScript Compiler..."
if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    # Convert Windows path to Unix style for Git Bash
    INSTALL_DIR=$(cygpath -u "$INSTALL_DIR")
    if ! copy_with_retry "target/release/runescript-compiler.exe" "$INSTALL_DIR/bin/rsc.exe"; then
        exit 1
    fi
else
    if ! copy_with_retry "target/release/runescript-compiler" "$INSTALL_DIR/bin/rsc"; then
        exit 1
    fi
fi

# Function to add configuration to a shell rc file
configure_rc_file() {
    local rc_file="$1"
    echo "Configuring $rc_file..."
    
    # Create file if it doesn't exist
    if [[ ! -f "$rc_file" ]]; then
        touch "$rc_file"
    fi
    
    # Remove any existing RSC configurations
    sed -i.bak '/# RuneScript Compiler/d' "$rc_file"
    sed -i.bak '/export PATH=.*\/.rsc\/bin/d' "$rc_file"
    sed -i.bak '/alias rsc=/d' "$rc_file"
    rm -f "${rc_file}.bak"
    
    # Add new configuration
    echo "" >> "$rc_file"
    echo "# RuneScript Compiler" >> "$rc_file"
    echo "export PATH=\"\$PATH:$INSTALL_DIR/bin\"" >> "$rc_file"
    
    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
        echo "alias rsc='$INSTALL_DIR/bin/rsc.exe'" >> "$rc_file"
    else
        echo "alias rsc='$INSTALL_DIR/bin/rsc'" >> "$rc_file"
    fi
}

# On Windows/Git Bash, configure both .zshrc and .bashrc
if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    configure_rc_file "$HOME/.zshrc"
    configure_rc_file "$HOME/.bashrc"
    
    echo "Installation complete! Please restart your terminal or run one of:"
    echo "For Zsh: source \"$HOME/.zshrc\""
    echo "For Bash: source \"$HOME/.bashrc\""
    
    # Source the appropriate RC file based on current shell
    if [[ -n "$ZSH_VERSION" ]]; then
        source "$HOME/.zshrc"
    elif [[ -n "$BASH_VERSION" ]]; then
        source "$HOME/.bashrc"
    fi
else
    # On Unix systems, detect and configure only the current shell
    if [[ -n "$ZSH_VERSION" ]]; then
        configure_rc_file "$HOME/.zshrc"
        source "$HOME/.zshrc"
        echo "Installation complete! Please restart your terminal or run:"
        echo "source \"$HOME/.zshrc\""
    else
        # Default to bash
        if [[ -f "$HOME/.bash_profile" ]]; then
            configure_rc_file "$HOME/.bash_profile"
            source "$HOME/.bash_profile"
            echo "Installation complete! Please restart your terminal or run:"
            echo "source \"$HOME/.bash_profile\""
        else
            configure_rc_file "$HOME/.bashrc"
            source "$HOME/.bashrc"
            echo "Installation complete! Please restart your terminal or run:"
            echo "source \"$HOME/.bashrc\""
        fi
    fi
fi 