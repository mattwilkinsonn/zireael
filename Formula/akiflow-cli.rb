class AkiflowCli < Formula
  desc "Command-line task management for Akiflow (fork of code-yeongyu/akiflow-cli)"
  homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/akiflow-cli"
  version "0.3.10"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-darwin-arm64.tar.gz"
      # SHA256 is bumped by .github/workflows/release.yml when a tag is pushed.
      sha256 "75ee52b73daf1c4a2a1321c2c64efd8a597906463402c6464f3fd0038d752c20"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-x64.tar.gz"
      sha256 "a78a74f7f3ce2968cea7e835301d88a0ad2d53a0ec9788b9114da9b49832824b"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-arm64.tar.gz"
      sha256 "edfc83740c51891210ab44d6f683f374a7566a1650311399b3b9c0a5f2378dbd"
    end
  end

  def install
    # Binary is named `af` per the upstream — install it under that name.
    bin.install "af"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/af --version")
  end
end
