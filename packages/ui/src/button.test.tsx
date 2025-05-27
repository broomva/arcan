import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { Button } from './button';

describe('Button', () => {
  it('renders with children', () => {
    render(<Button appName="TestApp">Click me</Button>);
    expect(screen.getByText('Click me')).toBeInTheDocument();
  });

  it('applies custom className', () => {
    render(<Button appName="TestApp" className="custom-class">Test</Button>);
    const button = screen.getByText('Test');
    expect(button.className).toContain('custom-class');
  });

  it('triggers alert with appName on click', () => {
    const alertSpy = vi.spyOn(window, 'alert').mockImplementation(() => {});
    render(<Button appName="TestApp">Button</Button>);
    const button = screen.getByText('Button');
    button.click();
    expect(alertSpy).toHaveBeenCalledWith('Hello from your TestApp app!');
    alertSpy.mockRestore();
  });
}); 