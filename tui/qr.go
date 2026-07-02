package tui

import (
    "fmt"
    qrcode "github.com/skip2/go-qrcode"
)

// GenerateQR returns a string of the QR code for the given data
func GenerateQR(data string) (string, error) {
    qr, err := qrcode.New(data, qrcode.Medium)
    if err != nil {
        return "", fmt.Errorf("qr generate: %w", err)
    }
    // Convert to ASCII art
    art := qr.ToString(true)
    return art, nil
}
