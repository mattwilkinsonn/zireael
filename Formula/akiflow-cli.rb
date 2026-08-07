class AkiflowCli < Formula
  desc "Command-line task management for Akiflow (fork of code-yeongyu/akiflow-cli)"
  homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/akiflow-cli"
  version "0.3.8"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-darwin-arm64.tar.gz"
      # SHA256 is bumped by .github/workflows/release.yml when a tag is pushed.
      sha256 "23ac595011b6aa00d9f05d833dcc7aba36d4447ac611cc5d3ee86446ee3a60f2"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-x64.tar.gz"
      sha256 "7facb18be2a4a794d8f4d7370d4b6b118a97391583bf34f5daca018a5ce87369"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-arm64.tar.gz"
      sha256 "16b2745880ebbf47a07d49d51c6f8eba2261ddefd40d08b994ced4a01b9752b5"
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
