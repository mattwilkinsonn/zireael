class AkiflowCli < Formula
  desc "Command-line task management for Akiflow (fork of code-yeongyu/akiflow-cli)"
  homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/akiflow-cli"
  version "0.3.11"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-darwin-arm64.tar.gz"
      # SHA256 is bumped by .github/workflows/release.yml when a tag is pushed.
      sha256 "c420d1b9fa2bfdb2b43e967ad45e6a7c1bee8d8968fff4f91ab4640d3afc32c3"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-x64.tar.gz"
      sha256 "dc61ec1134e4e6caca76eb60c3f5ec41f15afcfd993a5887107a9c488e55709a"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-arm64.tar.gz"
      sha256 "2a8919b5e846d081f9d6cf31f8d37e9a20178675ea7db347674dd25de59f7047"
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
