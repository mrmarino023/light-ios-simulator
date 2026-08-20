# typed: false
# frozen_string_literal: true

# Install:
#   brew install --HEAD ./Formula/ligh.rb
# Or:
#   brew install --HEAD https://github.com/mrmarino023/light-ios-simulator.git

class Ligh < Formula
  desc "GPU-native Rust host for real iOS Simulator development (no Simulator.app)"
  homepage "https://github.com/mrmarino023/light-ios-simulator"
  license "MIT"
  head "https://github.com/mrmarino023/light-ios-simulator.git", branch: "main"

  depends_on "rust" => :build
  depends_on :macos
  depends_on :xcode

  def install
    system "cargo", "install", "--path", "crates/ligh-cli", "--root", prefix, "--locked"
    system "cargo", "install", "--path", "crates/ligh-daemon", "--root", prefix, "--locked"
  end

  def caveats
    <<~EOS
      Requires Xcode + an iOS Simulator runtime.

        ligh doctor
        ligh gui --device iphone-15-pro
        ligh probe --device iphone-15-pro

      Uses private Apple frameworks — may need updates per Xcode release.
    EOS
  end

  test do
    assert_match "ligh", shell_output("#{bin}/ligh --version")
  end
end
