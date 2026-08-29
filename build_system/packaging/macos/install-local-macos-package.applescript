on run argv
    if (count of argv) is not 2 then
        error "usage: install-local-macos-package.applescript <package> <target>"
    end if

    set packagePath to item 1 of argv
    set targetPath to item 2 of argv
    set installCommand to "/usr/sbin/installer -pkg " & quoted form of packagePath & " -target " & quoted form of targetPath
    do shell script installCommand with administrator privileges
end run
