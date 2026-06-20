class AkiflowCli < Formula
  desc "Command-line task management for Akiflow (fork of code-yeongyu/akiflow-cli)"
  homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/akiflow-cli"
  version "0.3.6"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-darwin-arm64.tar.gz"
      # SHA256 is bumped by .github/workflows/release.yml when a tag is pushed.
      sha256 "9d7e966b5df27827ffd84c65119d14509e94a3b937b9f8aa695e98cd1fd3956c"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-x64.tar.gz"
      sha256 "d6430c0db2b8334e0f5850ec6c9144f68d3b7aabbe637c99c157e2a4d27ad114"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-arm64.tar.gz"
      sha256 "b05eeb9f317dcc611c20dc501516a3198359337c59b79262635d41ef4f712202"
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
