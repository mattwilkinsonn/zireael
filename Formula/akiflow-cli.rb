class AkiflowCli < Formula
  desc "Command-line task management for Akiflow (fork of code-yeongyu/akiflow-cli)"
  homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/akiflow-cli"
  version "0.3.4"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-darwin-arm64.tar.gz"
      # SHA256 is bumped by .github/workflows/release.yml when a tag is pushed.
      sha256 "2a92f3b889fdf43e6cb69b7e1aebbfa171f3fa6eabbfac662366beaaefc7bf27"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-x64.tar.gz"
      sha256 "c4d4fb7c88f164a625113343a47b858d9b3ddecd2b7267ea1441fca58413b52c"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-arm64.tar.gz"
      sha256 "e2d9cbc6dd17d192e6addabf4418284e63147ce6410326140aaf7af05e992d68"
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
