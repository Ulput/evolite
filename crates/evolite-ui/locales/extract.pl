#!/usr/bin/env perl
# Collects the text of the interface into locales/_keys.json: every string literal marked
# `tr!("…")` in crates/evolite-ui/src and crates/evolite-core/src (what the core says to the
# person), outside tests. The marked text is English, and it is the key. A `format!`
# placeholder ({}, {name}, {x:?}) becomes {}; a `plural(n, tr!("a"), tr!("b"))` call becomes
# the key "{} a|{} b". The catalogs (locales/<code>.json) translate these keys; `cargo test`
# checks none is missing.
#
#   perl crates/evolite-ui/locales/extract.pl      (from the repository root)
use strict;
use warnings;
use utf8;
use File::Find;
binmode STDOUT, ':encoding(UTF-8)';

my %keys;
my @dirs = ('crates/evolite-ui/src', 'crates/evolite-core/src', 'crates/evolite-api/src');
my $lit = qr/"((?:[^"\\]|\\.)*)"/s;

find(sub {
    return unless /\.rs$/;
    my $path = $File::Find::name;
    return if $path =~ m{/i18n/} || $path =~ m{/tests?\.rs$} || $path =~ m{_tests\.rs$};
    # The installer has catalogs of its own (locales/setup/<code>.json) and its own check.
    return if $path =~ m{/src/setup/};
    open my $fh, '<:encoding(UTF-8)', $_ or die "$path: $!";
    local $/; my $src = <$fh>; close $fh;
    # What is under #[cfg(test)] — a test module or a test-only item anywhere in the file — is
    # not the interface; the code after it is.
    $src = without_tests($src);
    # Comments (line and doc) carry no interface text.
    $src =~ s{^\s*//[^\n]*}{}mg;
    # plural(n, tr!("a"), tr!("b")) → one key in both its forms.
    while ($src =~ /plural\(\s*[^,]+,\s*tr!\(\s*$lit\s*\)\s*,\s*tr!\(\s*$lit\s*\)\s*\)/g) {
        $keys{"{} " . unescape($1) . "|{} " . unescape($2)} = 1;
    }
    while ($src =~ /\btr!\(\s*$lit\s*\)/g) {
        my $key = unescape($1);
        # A format string: its placeholders become {} (named, positional, with a spec).
        $key =~ s/\{\{/\x{1}/g; $key =~ s/\}\}/\x{2}/g;
        $key =~ s/\{[A-Za-z0-9_.]*(?::[^{}]*)?\}/{}/g;
        $key =~ s/\x{1}/{/g; $key =~ s/\x{2}/}/g;
        $keys{$key} = 1;
    }
}, @dirs);

# `src` without each item marked #[cfg(test)]: from the mark to the brace that closes the item's
# first block (or to its `;`), skipping string and char literals, whose braces do not count.
sub without_tests {
    my ($src) = @_;
    my $out = '';
    while ($src =~ /#\[cfg\((?:all\()?test\b[^\]]*\]/g) {
        my $start = $-[0];
        my $i = $+[0];
        my $depth = 0;
        my $len = length $src;
        while ($i < $len) {
            my $c = substr($src, $i, 1);
            if ($c eq '"') {
                $i++;
                while ($i < $len && substr($src, $i, 1) ne '"') {
                    $i++ if substr($src, $i, 1) eq '\\';
                    $i++;
                }
            } elsif ($c eq "'" && substr($src, $i, 4) =~ /^'(\\.|[^\\'])'/) {
                $i += length($&) - 1;
            } elsif ($c eq '{') {
                $depth++;
            } elsif ($c eq '}') {
                $depth--;
                if ($depth == 0) { $i++; last }
            } elsif ($c eq ';' && $depth == 0) {
                $i++;
                last;
            }
            $i++;
        }
        $out .= substr($src, 0, $start);
        $src = $i < $len ? substr($src, $i) : '';
        pos($src) = 0;
    }
    return $out . $src;
}

sub unescape {
    my ($s) = @_;
    $s =~ s/\\\n\s*//g;            # string continuation
    $s =~ s/\\n/\n/g; $s =~ s/\\t/\t/g; $s =~ s/\\"/"/g; $s =~ s/\\\\/\\/g;
    $s =~ s/\\u\{([0-9a-fA-F]+)\}/chr(hex($1))/ge;
    return $s;
}

# The plural forms are keys of their own; their single forms are not needed again.
my %forms = map { $_ => 1 } map { my $k = $_; $k =~ s/\{\} //g; split /\|/, $k } grep { /\|/ } keys %keys;
delete $keys{$_} for keys %forms;

open my $out, '>:encoding(UTF-8)', 'crates/evolite-ui/locales/_keys.json' or die $!;
my @sorted = sort keys %keys;
print $out "[\n", join(",\n", map { '  ' . json($_) } @sorted), "\n]\n";
close $out;
printf "%d keys\n", scalar @sorted;

sub json {
    my ($s) = @_;
    $s =~ s/\\/\\\\/g; $s =~ s/"/\\"/g; $s =~ s/\n/\\n/g; $s =~ s/\t/\\t/g;
    return "\"$s\"";
}
