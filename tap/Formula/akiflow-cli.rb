class AkiflowCli < Formula
  desc "Akiflow CLI — task management from the command line (fork of code-yeongyu/akiflow-cli)"
  homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/akiflow-cli"
  license "MIT"
  version "0.3.0"

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-darwin-arm64.tar.gz"
      # SHA256 is bumped by .github/workflows/release.yml when a tag is pushed.
      sha256 "20f84d1884fa6c0f8ae398fa2fe5c0f2cea4d71a73b12fd924b70ceab4d1fd41"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-x64.tar.gz"
      sha256 "6686c41f3c80a24b0ab5f6b3424ee9f84c720fe6577953581cb72ed846a7a548"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/akiflow-cli-v#{version}-linux-arm64.tar.gz"
      sha256 "6ca04605f4f5a24789fbf10748b4d488e026df158214063bec93caef20e8696a"
    end
  end

  def install
    # Binary is named `af` per the upstream — install it under that name.
    bin.install "af"
  end

  test do
    assert_match "0.3.0", shell_output("#{bin}/af --version")
  end
end
